//! The cosmic-text shaping engine.
//!
//! Shapes through cosmic-text's shaping-only stage (`BufferLine::shape`, the path the
//! pre-Parley terminal code drove — no line breaking, no metrics hinting) and translates the
//! shaped glyphs into the engine-neutral model. `ShapeGlyph` advances/offsets/ascent are
//! em-relative (cosmic-text divides by the font scale at shape time), so the engine multiplies
//! by the requested `font_size` to get pixels.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{Attrs, AttrsList, BufferLine, FontSystem, LineEnding, Shaping, Weight};
use fontdb::Source;

use crate::engine::{
    FontBytes, FontData, FontRegistry, ShapedCluster, ShapedGlyph, ShapedRun, ShapingEngine,
    ShapingEngineKind, ShapingRequest, TextAttributes, TextFamily,
};
use crate::{FaceId, TextWeight};

/// The cosmic-text-backed [`ShapingEngine`].
///
/// Owns a cosmic-text [`FontSystem`] (locale, font database, shaping caches) plus a registry of
/// known faces. `fontdb::ID` is an opaque slotmap key, so the engine assigns its own sequential
/// registry indexes; those index into both a `fontdb::ID` map (for shaping) and a font-data
/// map (for rasterization through `FaceId`). Fallback faces the shaper selects without an
/// explicit `load_font` call are interned lazily on first use.
///
/// The engine holds no internal lock: instances are owned by [`crate::FontManager`], whose
/// single outer mutex already serializes all access; locking again here would only add a
/// second, uncontended layer.
pub struct CosmicTextEngine {
    font_system: FontSystem,
    /// One entry per known face, in registration order. The registry index is the [`FaceId`]
    /// payload. The map is published as an immutable `Arc` snapshot after every mutation
    /// (registration and lazy fallback interning); see [`ShapingEngine::font_registry`].
    faces: Vec<CosmicFace>,
    /// The `faces` snapshot last published as an `Arc`. Rebuilt on every mutation:
    /// interning is rare after the first use of a fallback face, so the O(n) clone is
    /// acceptable; readers (the manager's published snapshot) stay clone-cheap.
    /// Simplification note: the separate Vec could go if `faces` were itself an Arc, at
    /// the cost of copy-on-write for intern lookups — kept simple for now.
    published_faces: Arc<Vec<CosmicFace>>,
    /// Faces the shaper resolved without a registry hit, mapped by their `fontdb::ID` so
    /// the (byte-copying) data read and content comparison happens at most once per
    /// distinct face — not per glyph of every cluster, every frame.
    resolved: HashMap<fontdb::ID, FaceId>,
}

impl CosmicTextEngine {
    /// Rebuild `published_faces` from the database after a mutation. Call sites:
    /// `register` and `intern` — the only two growth points. `pull` reuses the caller's
    /// published snapshot instead of re-deriving one (see below).
    fn publish(&mut self) {
        self.published_faces = Arc::new(self.faces.clone());
    }

    /// Create a shape-ready clone from the published registry (the cosmic *epoch-pull*,
    /// ADR 0006).
    ///
    /// The `FontSystem` is built through [`FontSystem::new_with_locale_and_db`] over a
    /// database holding **exactly the published registry's faces** — every other public
    /// constructor runs `db.load_system_fonts()` internally (cosmic-text 0.19), which
    /// would (a) cost a full system scan per seed and (b) pollute the session database
    /// with system copies of loaded families. The pollution was the white-screen bug:
    /// the shaper resolved the *system* copy of the terminal font, whose db id the
    /// registry never knew, forcing every glyph through fallback-mint resolution. With a
    /// registry-only database every shaper selection is a registry face by construction:
    /// `lookup` hits directly, `FaceId`s are stable, and minting only ever happens for
    /// genuine fallback selections (unknown scripts, emoji).
    pub(crate) fn seed_from_registry(published: &FontRegistry) -> Self {
        let mut engine = Self {
            font_system: Self::registry_only_font_system(),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
            resolved: HashMap::new(),
        };
        engine.pull(published);
        engine
    }

    /// The scan-free `FontSystem` constructor the session seed uses (see
    /// `seed_from_registry`): the locale is read the same way cosmic-text does
    /// (`sys_locale`, defaulted to `en-US`), and the database starts empty — `pull`
    /// loads the published registry's faces into it right after.
    fn registry_only_font_system() -> FontSystem {
        // `FontSystem::get_locale` is private cosmi 0.19; mirror its std default here.
        let locale = "en-US".to_string();
        FontSystem::new_with_locale_and_db(locale, fontdb::Database::new())
    }

    /// Incrementally bring this engine in line with the published snapshot: load every
    /// registry entry the engine has not seen yet (payloads are sequential mint-order, so
    /// "beyond this engine's face count" identifies exactly the new faces) and return
    /// whether anything was loaded.
    ///
    /// Called by the manager at session-open sync; cheap when the world has not moved.
    pub(crate) fn pull(&mut self, published: &FontRegistry) -> bool {
        let fresh = published
            .entries()
            .into_iter()
            .filter(|(id, _)| id.payload() as usize >= self.faces.len())
            .collect::<Vec<_>>();
        if fresh.is_empty() {
            return false;
        }
        for (id, data) in fresh {
            debug_assert_eq!(
                id.payload() as usize,
                self.faces.len(),
                "seed replay must preserve the published FaceId payloads (registry order)"
            );
            let source =
                Source::Binary(Arc::clone(&data.data) as Arc<dyn AsRef<[u8]> + Send + Sync>);
            let db_ids = self.font_system.db_mut().load_font_source(source);
            let db_id = *db_ids.first().expect("a registered font file yields faces");
            let face_info = self.font_system.db().face(db_id).expect("just-loaded face");
            self.faces.push(CosmicFace {
                id: db_id,
                // The weight is shaping-lookup metadata only; the published data carries
                // the full identity, and the default is what a fresh registration used.
                weight: face_info.weight,
                data: Arc::clone(&data.data),
                data_index: data.index,
            });
        }
        self.publish();
        true
    }

    /// Shape with fallback-face resolution routed through a caller-chosen policy (ADR 0006):
    /// the canonical engine interns into its own registry (trait `shape`), session-path
    /// clones mint through the manager — the single `FaceId` authority — so every `FaceId`
    /// minted anywhere is globally valid and the manager's published snapshot carries the
    /// face at mint time.
    pub(crate) fn shape_with_resolver(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        resolve: &dyn Fn(&mut Self, fontdb::ID, fontdb::Weight) -> Option<FaceId>,
    ) -> Option<ShapedRun> {
        self.shape_impl(request, font_size, resolve)
    }
}

/// Snapshots published through [`ShapingEngine::font_registry`] must be shareable; the
/// registry clone in `publish` needs face entries to be cloneable (they are shared bytes).
#[derive(Clone)]
struct CosmicFace {
    id: fontdb::ID,
    /// The weight the face was registered/selected with; part of the shaping lookup key.
    weight: fontdb::Weight,
    /// A strong reference to the font bytes, so `font_data` can resolve without re-reading.
    data: FontBytes,
    data_index: u32,
}

impl std::fmt::Debug for CosmicTextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmicTextEngine")
            .field("font_count", &self.faces.len())
            .finish_non_exhaustive()
    }
}

impl CosmicTextEngine {
    /// A bare engine over the given engine kind: no fallbacks, no fonts.
    pub fn bare() -> Self {
        Self {
            font_system: Self::registry_only_font_system(),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
            resolved: HashMap::new(),
        }
    }

    /// Create an engine with the environment's locale, system fonts, and fallbacks loaded.
    pub fn system() -> Self {
        Self {
            font_system: FontSystem::new(),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
            resolved: HashMap::new(),
        }
    }
}

impl ShapingEngine for CosmicTextEngine {
    fn name(&self) -> &'static str {
        "cosmic-text"
    }

    fn load_font(&mut self, data: FontBytes) -> Vec<FaceId> {
        self.register(data)
    }

    fn font_data(&self, id: FaceId) -> Option<FontData> {
        let face = self.faces.get(id.payload() as usize)?;
        Some(FontData {
            data: Arc::clone(&face.data),
            index: face.data_index,
        })
    }

    fn font_registry(&self) -> Arc<FontRegistry> {
        let fonts: HashMap<FaceId, FontData> = self
            .published_faces
            .iter()
            .enumerate()
            .map(|(index, face)| {
                (
                    FaceId::new(index as u64),
                    FontData {
                        data: Arc::clone(&face.data),
                        index: face.data_index,
                    },
                )
            })
            .collect();
        let metrics = crate::face_metrics::extract_all(&fonts);
        Arc::new(FontRegistry::from_owned(fonts, metrics))
    }

    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        // Canonical-engine path (see `shape_with_resolver`): unregistered fallback faces
        // intern into this engine's own registry.
        self.shape_with_resolver(request, font_size, &|engine, id, weight| {
            engine.intern(id, weight).map(Self::face_id)
        })
    }

    fn mint_face(&mut self, data: FontData) -> Option<FaceId> {
        // Dedupe by content: session-path shaping resolves fontdb faces (system-scanned
        // duplicates included) whose data may already be registered — e.g. the terminal
        // font loaded via `load_font` matched in the session db as the system-scanned
        // copy of the same family. Minting would append a duplicate under a *fresh*
        // FaceId every time, and per-frame registry snapshots (read before the session
        // opened) would never carry it — every cluster's metrics lookup would miss and
        // the glyphs would never render. The same font data is always the same `FaceId`.
        // The same font data is always the same `FaceId`. Fast path: the identical
        // allocation (a pulled registry face reshaped through the session db); fallback:
        // byte compare — db-resolved faces hand out a fresh copy per read, so ptr_eq
        // alone would miss byte-identical data.
        let bytes = data.data.as_ref().as_ref();
        if let Some(index) = self.faces.iter().position(|face| {
            face.data_index == data.index
                && (Arc::ptr_eq(&face.data, &data.data) || face.data.as_ref().as_ref() == bytes)
        }) {
            return Some(Self::face_id(index));
        }
        // The id here is mint-order sequential; the caller's data must already be globally
        // unambiguous — see `face_data` read in the session resolver.
        let registered = self.register(data.data)[0];
        Some(registered)
    }
}

impl CosmicTextEngine {
    /// Shape through this engine's `FontSystem`; faces the shaper selects without a
    /// registered entry are resolved by `resolve` — the canonical engine's trait `shape`
    /// interns locally, per-clone sessions mint through the manager (see the ADR 0006
    /// callers of this method).
    fn shape_impl(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        resolve: &dyn Fn(&mut Self, fontdb::ID, fontdb::Weight) -> Option<FaceId>,
    ) -> Option<ShapedRun> {
        // The shaping-only path: no line breaking, no metrics hinting. When the metadata
        // mechanism is enabled, metadata flows through cosmic-text natively (Attrs::metadata
        // is copied onto each ShapeGlyph), so no per-cluster lookup is needed here.
        let mut attrs_list = AttrsList::new(&attrs(&request.default_attributes));
        for (range, attributes) in &request.ranges {
            attrs_list.add_span(range.clone(), &attrs(attributes));
        }

        let mut buffer = BufferLine::new(
            request.text,
            LineEnding::None,
            attrs_list,
            Shaping::Advanced,
        );
        let shape_line = buffer.shape(&mut self.font_system, 0);

        let mut max_ascent = 0.0_f32;
        let mut max_descent = 0.0_f32;
        let mut width = 0.0_f32;
        // Sized exactly before building (this engine emits one cluster per ShapeGlyph, so the
        // count serves both arrays; the recount is cheap next to shaping itself).
        let glyph_count: usize = shape_line
            .spans
            .iter()
            .flat_map(|span| &span.words)
            .map(|word| word.glyphs.len())
            .sum();
        let mut glyphs: Vec<ShapedGlyph> = Vec::with_capacity(glyph_count);
        let mut clusters: Vec<ShapedCluster> = Vec::with_capacity(glyph_count);

        for span in &shape_line.spans {
            for word in &span.words {
                // A word's glyphs share one line position; the cluster origin is the line
                // advance so far (plus the glyph's own x offset) and glyph x stays intra-cluster
                // so the `GlyphRun` left-bearing logic works engine-independently.
                for glyph in &word.glyphs {
                    // ShapeGlyph units are em-relative; scale to pixels.
                    let glyph_px = font_size * glyph.x_advance;
                    let offset_px = font_size * glyph.x_offset;
                    // cosmic-text copies harfbuzz's Y-up `y_offset` verbatim into
                    // `ShapeGlyph`; negate into the shared Y-down convention (see
                    // `ShapedGlyph::y`). Parley does the same conversion upstream.
                    let y_px = -font_size * glyph.y_offset;

                    // A face not yet in this engine's registry: resolve through the
                    // engine's own intern (canonical sessions) or the manager mint
                    // (per-clone sessions, ADR 0006). Either policy yields a globally
                    // valid id and records the face in the published world.
                    let face_id = match self.lookup(glyph.font_id, glyph.font_weight) {
                        Some(index) => Self::face_id(index),
                        // First sighting of this database face: resolve through `resolve`
                        // (canonical intern or manager mint), then memoize — the resolver
                        // reads and compares whole font files, unaffordable per glyph.
                        None => match self.resolved.get(&glyph.font_id) {
                            Some(face_id) => *face_id,
                            None => {
                                let face_id = resolve(self, glyph.font_id, glyph.font_weight)?;
                                self.resolved.insert(glyph.font_id, face_id);
                                face_id
                            }
                        },
                    };

                    let cluster_x = width + offset_px;
                    max_ascent = max_ascent.max(font_size * glyph.ascent);
                    max_descent = max_descent.max(font_size * glyph.descent);
                    width += glyph_px;

                    // One ShapeGlyph per cluster, so each range is exactly one slot appended
                    // contiguously to the run's flat glyph array.
                    let glyph_range = glyphs.len() as u32..glyphs.len() as u32 + 1;
                    glyphs.push(ShapedGlyph {
                        glyph_id: glyph.glyph_id,
                        face_id,
                        font_size,
                        weight: TextWeight(glyph.font_weight.0),
                        // Intra-cluster: this engine emits one glyph per cluster, so the
                        // glyph's own offset is 0 relative to the cluster origin.
                        x: 0.0,
                        y: y_px,
                    });
                    clusters.push(ShapedCluster {
                        byte_range: glyph.start..glyph.end,
                        x: cluster_x,
                        advance: glyph_px,
                        // Cosmic-text propagates Attrs metadata through shaping (composed
                        // clusters like base+mark carry their first byte's span), so the echo
                        // matches `covering_metadata` by construction (0 when disabled: the
                        // spans are still built without metadata).
                        metadata: if request.metadata { glyph.metadata } else { 0 },
                        glyph_range,
                    });
                }
            }
        }

        Some(ShapedRun {
            glyphs,
            clusters,
            max_ascent,
            max_descent,
            width,
            engine: ShapingEngineKind::CosmicText,
        })
    }

    /// Register a font file's faces into the database and the registry.
    fn register(&mut self, data: FontBytes) -> Vec<FaceId> {
        // fontdb holds its own shared reference to the bytes.
        let source = Source::Binary(Arc::clone(&data) as Arc<dyn AsRef<[u8]> + Send + Sync>);
        let db_ids = self.font_system.db_mut().load_font_source(source);
        let ids: Vec<FaceId> = db_ids
            .into_iter()
            .map(|id| {
                let face_info = self.font_system.db().face(id).expect("just-loaded face");
                let index = self.faces.len();
                self.faces.push(CosmicFace {
                    id,
                    weight: face_info.weight,
                    data: Arc::clone(&data),
                    data_index: face_info.index,
                });
                Self::face_id(index)
            })
            .collect();
        self.publish();
        ids
    }

    /// The registry index of a database face already known to this engine, if any.
    fn lookup(&self, id: fontdb::ID, weight: fontdb::Weight) -> Option<usize> {
        self.faces
            .iter()
            .position(|face| face.id == id && face.weight == weight)
    }

    /// Intern a shaper-selected (possibly fallback) `fontdb::ID` into this engine's registry
    /// on first use. The face's bytes are read out of the database once and pinned so
    /// rasterization resolves the same data later; the pre-ADR-0006 behavior this restores
    /// (canonical-engine resolution; per-clone sessions resolve through the manager
    /// instead). Its `None` on unreadable face data aborts the run — regression-covered in
    /// `tests.rs` (`cosmic_engine_shapes_through_shared_file_faces`).
    pub(crate) fn intern(&mut self, id: fontdb::ID, weight: fontdb::Weight) -> Option<usize> {
        let Some(index) = self.lookup(id, weight) else {
            // Robustness: `SharedFile` sources hand out their bytes as an Arc we can read
            // here; only an unreadable `Source::File` (deleted/unreadable path) fails.
            let (data, data_index) = self
                .font_system
                .db()
                .with_face_data(id, |bytes, face_index| {
                    (Arc::new(bytes.to_vec()) as FontBytes, face_index)
                })?;
            let face_weight = self
                .font_system
                .db()
                .face(id)
                .map(|face| face.weight)
                .unwrap_or(weight);
            let index = self.faces.len();
            self.faces.push(CosmicFace {
                id,
                weight: face_weight,
                data,
                data_index,
            });
            self.publish();
            return Some(index);
        };
        Some(index)
    }

    /// Read a database face's raw font data plus its face index (for the manager-mint
    /// seam, ADR 0006): the same read the canonical `intern` performs, without growing any
    /// registry. Returns `None` only for unreadable sources (see `intern`).
    pub(crate) fn face_data(&self, id: fontdb::ID) -> Option<FontData> {
        self.font_system
            .db()
            .with_face_data(id, |bytes, face_index| FontData {
                data: Arc::new(bytes.to_vec()) as FontBytes,
                index: face_index,
            })
    }

    fn face_id(index: usize) -> FaceId {
        FaceId::new(index as u64)
    }
}

#[cfg(test)]
mod tests;

/// Build cosmic-text attrs for one attributes value.
///
/// The returned `Attrs` borrows the family name for as long as `attributes` is borrowed; the
/// caller shapes within that lifetime (`BufferLine::new` copies the text but the attr list keeps
/// the family borrow until shaping finishes).
fn attrs<'a>(attributes: &'a TextAttributes<'_>) -> Attrs<'a> {
    let base = Attrs::new()
        .weight(Weight(attributes.weight.0))
        // Carry the caller's attribute identity through shaping; cosmic-text copies this onto
        // every ShapeGlyph. Always set (not conditioned on the request flag): the value is 0
        // for all non-metadata requests, and reading it back is free. Shapes each span
        // independently at these attrs, so a cluster never crosses an attribute boundary.
        .metadata(attributes.metadata);
    match &attributes.family {
        TextFamily::Named(name) => base.family(cosmic_text::Family::Name(name.as_ref())),
        TextFamily::SansSerif => base.family(cosmic_text::Family::SansSerif),
        TextFamily::Serif => base.family(cosmic_text::Family::Serif),
        TextFamily::Monospace => base.family(cosmic_text::Family::Monospace),
    }
}
