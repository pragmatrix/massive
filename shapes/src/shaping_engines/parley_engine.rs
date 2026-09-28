//! The Parley (fontique + harfrust) shaping engine.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use parley::fontique::{
    self, Blob, Collection, CollectionOptions, FamilyId, GenericFamily, SourceCache,
};
use parley::{
    FontContext, FontFamily, FontFamilyName, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::engine::{
    EngineScratch, FontBytes, FontData, FontRegistry, ShapedCluster, ShapedGlyph, ShapedRun,
    ShapingEngine, ShapingEngineKind, ShapingRequest, covering_metadata,
};
use crate::font_validation::validate_font_file;
use crate::shaping_engines::parley_scratch::ParleyScratch;
use crate::{FaceId, TextFamily, TextWeight};

/// Default Parley brush type (RGBA bytes). Callers overwrite color via `GlyphRun::with_color`.
type GlyphBrush = [u8; 4];

/// The Parley-backed [`ShapingEngine`].
///
/// Owns the Parley font database plus a registry of [`parley::FontData`] entries keyed by
/// [`FaceId`] (the `Blob` unique id plus the face index). A [`FaceId`] is derived straight from
/// a shaped run's font, so shaping needs no lookup; rasterization resolves a [`FaceId`] back to
/// concrete font data in O(1).
///
/// The engine holds no internal lock: instances are owned by [`crate::FontManager`], whose
/// single outer mutex already serializes all access; locking again here would only add a
/// second, uncontended layer.
pub struct ParleyEngine {
    font_context: FontContext,
    layout_context: LayoutContext<GlyphBrush>,
    /// Concrete fonts keyed by [`FaceId`]. Populated by `rebuild_fonts` to include every font the
    /// collection may select (including system fallbacks like emoji), so rasterization can resolve
    /// any glyph's `FaceId` to font data. The key must include the face index because a single
    /// file may hold several faces that share one `Blob` id.
    ///
    /// Immutable snapshot: the map itself is replaced (new `Arc`) on every mutation, so the
    /// manager can publish it lock-free (see [`ShapingEngine::font_registry`]). Static after
    /// startup: only `rebuild_fonts` and `load_font` write here — shaping never resolves faces,
    /// so a published snapshot cannot go stale while instances shape.
    fonts: HashMap<FaceId, FontData>,
}

impl fmt::Debug for ParleyEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParleyEngine")
            .field("font_count", &self.fonts.len())
            .finish_non_exhaustive()
    }
}

impl ParleyEngine {
    /// A collection in fontique's **shared mode** (ADR 0006): all clones share state behind
    /// an internal mutex with a version counter, so a font registered at any time becomes
    /// visible to every clone on its next read (`query` syncs lazily on version mismatch).
    /// Sharing is what lets per-task shaping contexts (see `parley_session_contexts`) work
    /// over the manager's one collection without replica/broadcast machinery.
    pub fn shared_collection(options: CollectionOptions) -> Collection {
        Collection::new(CollectionOptions {
            shared: true,
            ..options
        })
    }

    /// Create an engine with or without system fonts.
    ///
    /// Returns an error only if a catalog source cannot be read; a file fontique cannot parse is
    /// skipped during collection construction and so cannot fail engine construction.
    pub fn new(system_fonts: bool) -> Result<Self> {
        let mut font_context = FontContext {
            collection: Self::shared_collection(CollectionOptions {
                system_fonts,
                ..Default::default()
            }),
            // Parley creates an unshared source cache by default and prunes it on every layout
            // builder creation. A pruned font file is re-loaded on demand with a NEW `Blob` id,
            // which invalidates `FaceId`s derived from it (the renderer would see unknown faces).
            // The shared cache stores only weak blob refs and is never pruned; the registry built
            // by `rebuild_fonts` pins strong refs, so a pruned entry always upgrades back to the
            // original blob and `Blob` ids stay stable for the engine's lifetime.
            source_cache: SourceCache::new_shared(),
        };

        if system_fonts {
            font_context.collection.load_system_fonts();
        }

        let mut engine = Self::from_context(font_context);
        if system_fonts {
            // The registry and the symbol-fallback repair both derive from the collection Parley
            // may select from; a bare engine has no collection to walk.
            engine.rebuild_fonts();
            engine.validate_system_fonts()?;
        }
        Ok(engine)
    }

    /// Fail if a catalog source cannot be read; a source fontique could not parse is not a
    /// failure, because it contributed no face to the collection in the first place.
    ///
    /// Every registered face is Swash-readable: fontique registers a face only if the file
    /// carries a supported sfnt tag and the face's name and cmap tables parse, which is strictly
    /// stronger than Swash's readability check on that face. Re-checking the file here would also
    /// condemn a face fontique already skipped — a sibling in the same collection file — and take
    /// the whole catalog down with it.
    fn validate_system_fonts(&mut self) -> Result<()> {
        let family_names: Vec<String> = self
            .font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        for name in family_names {
            let Some(family_id) = self.font_context.collection.family_id(&name) else {
                continue;
            };
            let Some(family) = self.font_context.collection.family(family_id) else {
                continue;
            };
            for font_info in family.fonts() {
                if self
                    .font_context
                    .source_cache
                    .get(font_info.source())
                    .is_none()
                {
                    log::warn!(
                        "Rejecting Parley system-font catalog: cannot read {name} face index {}",
                        font_info.index()
                    );
                    return Err(anyhow!(
                        "cannot read Parley system font {name} face index {}",
                        font_info.index()
                    ));
                }
            }
        }
        Ok(())
    }

    fn from_context(font_context: FontContext) -> Self {
        Self {
            font_context,
            layout_context: LayoutContext::new(),
            fonts: HashMap::new(),
        }
    }

    /// Rebuild the font registry from the whole collection, keyed by [`FaceId`], so any font
    /// Parley may select (including system fallbacks like emoji) can be resolved by [`FaceId`]
    /// during rasterization. The strong `Blob` refs held here also keep the shared source
    /// cache's weak refs alive, so pruned entries re-resolve to the original blobs.
    fn rebuild_fonts(&mut self) {
        let fonts = Self::collect_fonts(&mut self.font_context);
        self.fonts = fonts;

        // Common-script symbols (e.g. `✘`, `✓`, `→`) inherit the surrounding script for fallback,
        // which on macOS resolves to Helvetica — a font that lacks most of them. Append the system
        // font with the best coverage of these symbols to the Latin fallback so they render instead
        // of falling through to the `.notdef` dead glyph. This is a local workaround for the known
        // upstream gap (parley #744, #695) until font selection becomes coverage-aware.
        Self::append_symbol_fallback(&mut self.font_context);
    }

    /// Collect the font registry from the collection: one [`FontData`] per (blob, face index),
    /// so any font Parley may select resolves by [`FaceId`]. Split from `rebuild_fonts` so the
    /// collection borrow ends before the registry is installed.
    fn collect_fonts(font_context: &mut FontContext) -> HashMap<FaceId, FontData> {
        let mut fonts = HashMap::new();
        let family_names: Vec<String> = font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        for name in family_names {
            let Some(family_id) = font_context.collection.family_id(&name) else {
                continue;
            };
            let Some(family) = font_context.collection.family(family_id) else {
                continue;
            };
            for font_info in family.fonts() {
                let Some(blob) = font_context.source_cache.get(font_info.source()) else {
                    continue;
                };
                let parley_font = parley::FontData::new(blob, font_info.index());
                let id = face_id_from_parley_data(&parley_font);
                // The Blob keeps the font bytes alive; hand its backing Arc to the neutral
                // registry so rasterization reads the same bytes without a copy.
                let (arc, _) = parley_font.data.into_raw_parts();
                let font = FontData::new(arc, font_info.index());
                fonts.insert(id, font);
            }
        }
        fonts
    }

    /// Append symbol-covering non-emoji fonts to the Latin fallback, best coverage first.
    ///
    /// Common-script symbols inherit the surrounding script (Latin by default) for fallback, but
    /// the platform's Latin fallback often lacks them. We scan a broad set of symbol codepoints,
    /// score every family by how many it covers, and append the covering families to the Latin
    /// fallback in descending coverage order. Emoji fonts are excluded because they would be
    /// selected for text-presentation symbols (upstream parley #744).
    fn append_symbol_fallback(font_context: &mut FontContext) {
        use fontique::{FallbackKey, GenericFamily, Script};

        // Common-script symbol blocks commonly used in terminals and UI text.
        const SYMBOL_RANGES: &[(u32, u32)] = &[
            (0x2000, 0x206F), // General Punctuation
            (0x2190, 0x21FF), // Arrows
            (0x2200, 0x22FF), // Mathematical Operators
            (0x2300, 0x23FF), // Miscellaneous Technical
            (0x2500, 0x257F), // Box Drawing
            (0x2580, 0x259F), // Block Elements
            (0x25A0, 0x25FF), // Geometric Shapes
            (0x2600, 0x26FF), // Miscellaneous Symbols
            (0x2700, 0x27BF), // Dingbats
            (0x27C0, 0x27EF), // Miscellaneous Mathematical Symbols-A
            (0x2980, 0x29FF), // Miscellaneous Mathematical Symbols-B
            (0x2B00, 0x2BFF), // Miscellaneous Symbols and Arrows
        ];

        let latn = Script::from_bytes(*b"Latn");
        let emoji_families: Vec<_> = font_context
            .collection
            .generic_families(GenericFamily::Emoji)
            .collect();

        // Score each family by how many symbol codepoints its default font covers.
        let family_names: Vec<String> = font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        let mut scored: Vec<(usize, FamilyId)> = Vec::new();
        for name in family_names {
            let Some(family_id) = font_context.collection.family_id(&name) else {
                continue;
            };
            if emoji_families.contains(&family_id) {
                continue;
            }
            let Some(family) = font_context.collection.family(family_id) else {
                continue;
            };
            let Some(font_info) = family.default_font() else {
                continue;
            };
            let Some(blob) = font_context.source_cache.get(font_info.source()) else {
                continue;
            };
            let Some(font_ref) =
                swash::FontRef::from_index(blob.as_ref(), font_info.index() as usize)
            else {
                continue;
            };
            let charmap = font_ref.charmap();
            let covered = SYMBOL_RANGES
                .iter()
                .flat_map(|&(start, end)| start..=end)
                .filter(|&c| charmap.map(char::from_u32(c).unwrap_or('\0')) != 0)
                .count();
            if covered > 0 {
                scored.push((covered, family_id));
            }
        }

        // Append best-coverage families first so the first that covers a symbol wins.
        scored.sort_by_key(|(covered, _)| std::cmp::Reverse(*covered));
        let families = scored.into_iter().map(|(_, id)| id);
        font_context
            .collection
            .append_fallbacks(FallbackKey::new(latn, None), families);
    }
}

impl ShapingEngine for ParleyEngine {
    fn name(&self) -> &'static str {
        "parley"
    }

    fn load_font(&mut self, data: FontBytes) -> Result<Vec<FaceId>> {
        if let Err(error) = validate_font_file(data.as_ref().as_ref()) {
            log::warn!("Rejecting Parley font file: {error}");
            return Err(error.into());
        }
        // FontData owns a shared `Blob<u8>`; keep the bytes alive in the registry.
        let blob: Blob<u8> = Blob::new(data);
        let families = self
            .font_context
            .collection
            .register_fonts(blob.clone(), None);
        // Register the newly loaded families as the generic families (sans-serif, serif, monospace)
        // so that text using a generic family name resolves to a font we actually have. Without
        // this, Parley would fall back to a system font for generic-family text, which may not be
        // in our registry and would fail to rasterize. Each generic is set only if it has no
        // existing mapping, so the first loaded font wins and later loads don't override it.
        for generic in [
            GenericFamily::SansSerif,
            GenericFamily::Serif,
            GenericFamily::Monospace,
        ] {
            if self
                .font_context
                .collection
                .generic_families(generic)
                .next()
                .is_none()
            {
                self.font_context
                    .collection
                    .set_generic_families(generic, families.iter().map(|(family, _)| *family));
            }
        }
        // A single font file (e.g. a `.ttc` collection) can hold several faces, each with its own
        // index. [`FaceId`] keys on the file's blob id *and* the face index, so one file yields one
        // [`FaceId`] per face — hence the nested loop and the multiple ids returned.
        let mut ids = Vec::new();
        for (_, faces) in families {
            for face in faces {
                let parley_font = parley::FontData::new(blob.clone(), face.index());
                let id = face_id_from_parley_data(&parley_font);
                let (arc, _) = parley_font.data.into_raw_parts();
                let font = FontData::new(arc, face.index());
                self.fonts.insert(id, font);
                ids.push(id);
            }
        }
        Ok(ids)
    }

    fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.fonts.get(&id).cloned()
    }

    fn font_registry(&self) -> Arc<FontRegistry> {
        Arc::new(FontRegistry::from_owned(self.fonts.clone()))
    }

    /// Shape one line through per-session contexts (ADR 0006): the same pipeline the
    /// canonical `shape` runs, parameterized so per-clone scratch state can shape over
    /// its own contexts.
    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        let Self {
            font_context,
            layout_context,
            ..
        } = self;
        shape_line(font_context, layout_context, request, font_size)
    }

    /// Per-context shaping scratch over this engine's shared collection (ADR 0006): a
    /// clone of the collection plus a fresh `LayoutContext`. A shared-mode collection
    /// clone shares the internally synchronized state, so later registrations (fonts
    /// loaded at any time) are visible to the scratch via fontique's version sync.
    fn new_scratch(&self, _published: &FontRegistry) -> Box<dyn EngineScratch> {
        Box::new(ParleyScratch::new(ParleySessionContexts {
            font_context: FontContext {
                collection: self.font_context.collection.clone(),
                source_cache: self.font_context.source_cache.clone(),
            },
            layout_context: LayoutContext::new(),
        }))
    }
}

/// Per-session shaping contexts over the engine's shared collection (ADR 0006).
///
/// A clone of the canonical engine's font collection plus a fresh `LayoutContext`; the
/// collection is in fontique shared mode, so registrations after the clone stay visible.
pub struct ParleySessionContexts {
    pub(crate) font_context: FontContext,
    pub(crate) layout_context: LayoutContext<GlyphBrush>,
}

/// Shape one attributed line through the given session contexts.
pub(crate) fn shape_line(
    font_context: &mut FontContext,
    layout_context: &mut LayoutContext<GlyphBrush>,
    request: &ShapingRequest<'_>,
    font_size: f32,
) -> Option<ShapedRun> {
    let mut builder = layout_context.ranged_builder(font_context, request.text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontFamily(parley_family(
        &request.default_attributes.family,
    )));
    builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(
        request.default_attributes.weight.0 as f32,
    )));
    for (range, attrs) in &request.ranges {
        builder.push(
            StyleProperty::FontFamily(parley_family(&attrs.family)),
            range.clone(),
        );
        builder.push(
            StyleProperty::FontWeight(parley::FontWeight::new(attrs.weight.0 as f32)),
            range.clone(),
        );
    }
    let mut layout: parley::Layout<GlyphBrush> = builder.build(request.text);
    layout.break_all_lines(None);
    layout.align(parley::Alignment::Start, Default::default());

    // Feature: Support multi-line layout.
    let line = layout.get(0)?;

    // Counting pre-pass sizes both arrays exactly; recounting shaped clusters is cheap next
    // to shaping itself.
    let (mut glyph_count, mut cluster_count) = (0, 0);
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            continue;
        };
        for cluster in glyph_run.run().clusters() {
            cluster_count += 1;
            glyph_count += cluster.glyphs().count();
        }
    }

    // Each shaped run carries its own font (fallback for emoji etc.), so `FaceId`/size/weight
    // are per run. Glyph y offsets are re-based onto the run baseline (Parley lays out
    // Y-down) so both engines share the `GlyphRun` convention. All clusters' glyphs append
    // contiguously to one flat run-level array; clusters carry index ranges into it.
    let mut glyphs: Vec<ShapedGlyph> = Vec::with_capacity(glyph_count);
    let mut clusters: Vec<ShapedCluster> = Vec::with_capacity(cluster_count);
    for item in line.items() {
        let glyph_run = match item {
            PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
            PositionedLayoutItem::InlineBox(_) => continue,
        };
        let run = glyph_run.run();
        let face_id = face_id_from_parley_data(run.font());
        let font_size = run.font_size();
        let weight = TextWeight(run.font_attrs().weight.value() as u16);
        let mut cluster_origin = glyph_run.offset();
        for cluster in run.clusters() {
            // Zero-glyph clusters are a real Parley artifact (e.g. a lone ZWJ between
            // letters) and would violate the "clusters are never empty" contract
            // consumers rely on (`cluster_glyphs(cluster)[0]`); skip them.
            if cluster.glyphs().count() == 0 {
                // The cluster still moves the line origin by its typographic advance.
                cluster_origin += cluster.advance();
                continue;
            }
            // The cluster's glyphs append to the flat array, in cluster order: the range is
            // the slice just appended.
            let glyph_start = glyphs.len() as u32;
            glyphs.extend(cluster.glyphs().map(|glyph| ShapedGlyph {
                glyph_id: glyph.id as u16,
                face_id,
                font_size,
                weight,
                x: glyph.x,
                y: glyph.y,
            }));
            clusters.push(ShapedCluster {
                byte_range: cluster.text_range(),
                // First-byte-cover echo, engine-neutrally defined in
                // `engine::covering_metadata`; shaping untouched (straddling
                // clusters like base+mark compositions resolve to their first
                // byte's range, the typographically correct side).
                metadata: if request.metadata {
                    covering_metadata(
                        &request.ranges,
                        request.default_attributes.metadata,
                        cluster.text_range().start,
                    )
                } else {
                    0
                },
                // Parley's cluster glyphs are intra-cluster relative; the cluster
                // origin accumulates the preceding clusters' advances on the line.
                x: cluster_origin,
                advance: cluster.advance(),
                glyph_range: glyph_start..glyphs.len() as u32,
            });
            cluster_origin += cluster.advance();
        }
    }

    let line_metrics = line.metrics();
    Some(ShapedRun {
        glyphs,
        clusters,
        max_ascent: line_metrics.ascent,
        max_descent: line_metrics.descent,
        width: line_metrics.advance,
        engine: ShapingEngineKind::Parley,
    })
}

fn parley_family<'a>(family: &TextFamily<'a>) -> FontFamily<'a> {
    match family {
        TextFamily::Named(name) => FontFamily::Single(FontFamilyName::Named(name.clone())),
        TextFamily::SansSerif => GenericFamily::SansSerif.into(),
        TextFamily::Serif => GenericFamily::Serif.into(),
        TextFamily::Monospace => GenericFamily::Monospace.into(),
    }
}

/// Derive a [`FaceId`] from Parley font data: the `Blob` unique id packed with the face index.
fn face_id_from_parley_data(font: &parley::FontData) -> FaceId {
    FaceId::new((font.data.id() << 32) | font.index as u64)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// One unreadable file in the catalog must not reject the whole catalog: fontique already
    /// skipped the face it could not parse, so every registered face is Swash-readable.
    #[test]
    fn one_unreadable_catalog_file_does_not_reject_the_catalog() {
        let mut engine = engine_with_font(database_with_a_partly_readable_file());

        assert_eq!(
            engine.fonts.len(),
            1,
            "fontique registers the valid face and skips the unreadable one"
        );
        assert!(
            engine.validate_system_fonts().is_ok(),
            "a file whose spare face is unreadable must not fail engine construction"
        );
    }

    /// A file fontique cannot parse at all contributes no face, so it cannot reject the catalog.
    #[test]
    fn unparseable_catalog_file_does_not_reject_the_catalog() {
        let mut engine = engine_with_font(b"not a font".to_vec());

        assert!(
            engine.fonts.is_empty(),
            "an unparseable file yields no face"
        );
        assert!(
            engine.validate_system_fonts().is_ok(),
            "an unparseable file must not fail engine construction"
        );
    }

    /// A catalog source that cannot be read must reject the catalog: the engine promises every
    /// published `FaceId` resolves to bytes, so a registered face whose file is gone is fatal.
    #[test]
    fn unreadable_catalog_source_rejects_the_catalog() {
        let path = write_catalog_font(JETBRAINS_MONO);
        let mut engine = ParleyEngine::from_context(FontContext {
            collection: ParleyEngine::shared_collection(CollectionOptions {
                system_fonts: false,
                ..Default::default()
            }),
            source_cache: SourceCache::new_shared(),
        });
        engine
            .font_context
            .collection
            .load_fonts_from_paths([&path]);
        let registered = engine.font_context.collection.family_names().count();
        std::fs::remove_file(&path).unwrap();

        // `rebuild_fonts` is the first read of the path, so it caches a failure and drops the
        // face from the registry; validation must then surface the unreadable source.
        engine.rebuild_fonts();

        assert!(registered > 0, "the readable file registered a face");
        assert!(
            engine.fonts.is_empty(),
            "the unreadable source contributed no face to the registry"
        );
        let error = engine
            .validate_system_fonts()
            .expect_err("an unreadable catalog source must fail engine construction");
        assert!(
            error.to_string().contains("cannot read"),
            "unexpected error: {error}"
        );
    }

    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// An engine with the given file registered as its only font, without system fonts.
    fn engine_with_font(bytes: Vec<u8>) -> ParleyEngine {
        let mut engine = ParleyEngine::from_context(FontContext {
            collection: ParleyEngine::shared_collection(CollectionOptions {
                system_fonts: false,
                ..Default::default()
            }),
            source_cache: SourceCache::new_shared(),
        });
        let blob: Blob<u8> = Blob::new(std::sync::Arc::new(bytes));
        engine.font_context.collection.register_fonts(blob, None);
        engine.rebuild_fonts();
        engine
    }

    /// A font database holding one valid face plus one whose table directory is garbage.
    ///
    /// Fontique registers a collection face by face, so the valid face is a selectable catalog
    /// entry whose bytes only parse because they are the whole file.
    fn database_with_a_partly_readable_file() -> Vec<u8> {
        const HEADER: usize = 20;
        let num_tables = u16::from_be_bytes([JETBRAINS_MONO[4], JETBRAINS_MONO[5]]) as usize;
        let mut ttc = Vec::with_capacity(JETBRAINS_MONO.len() + HEADER + 16);
        ttc.extend_from_slice(b"ttcf");
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&2u32.to_be_bytes());
        ttc.extend_from_slice(&(HEADER as u32).to_be_bytes());
        ttc.extend_from_slice(&(JETBRAINS_MONO.len() as u32 + HEADER as u32).to_be_bytes());
        ttc.extend_from_slice(JETBRAINS_MONO);
        for i in 0..num_tables {
            let offset = HEADER + 12 + 16 * i + 8;
            let old = u32::from_be_bytes(ttc[offset..offset + 4].try_into().unwrap());
            ttc[offset..offset + 4].copy_from_slice(&(old + HEADER as u32).to_be_bytes());
        }
        ttc.extend_from_slice(b"junk-table-dir!");
        ttc
    }

    /// A uniquely named `.ttf` in the temp dir holding `bytes`, so it can be registered from a
    /// [`fontique::SourceKind::Path`] source.
    fn write_catalog_font(bytes: &[u8]) -> PathBuf {
        static UNIQUE: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "massive-parley-catalog-{}-{}.ttf",
            std::process::id(),
            UNIQUE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
