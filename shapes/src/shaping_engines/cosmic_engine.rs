//! The cosmic-text shaping engine.
//!
//! Shapes through cosmic-text's shaping-only stage (`BufferLine::shape`, the path the
//! pre-Parley terminal code drove — no line breaking, no metrics hinting) and translates the
//! shaped glyphs into the engine-neutral model. `ShapeGlyph` advances/offsets/ascent are
//! em-relative (cosmic-text divides by the font scale at shape time), so the engine multiplies
//! by the requested `font_size` to get pixels.
//!
//! Default locale: cosmic-text derives its locale from the environment (`FontSystem::new`);
//! engines seeded from a prepared `fontdb` clone (`seed_font_system`, `bare()`) must construct
//! the `FontSystem` explicitly, so they mirror cosmic-text's std default `"en-US"` instead of
//! re-reading the private `FontSystem::get_locale` (private as of cosmic-text 0.19).

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use cosmic_text::{Attrs, AttrsList, BufferLine, FontSystem, LineEnding, Shaping, Weight};
use fontdb::Source;

use crate::engine::{
    EngineScratch, FontBytes, FontData, FontRegistry, ShapedCluster, ShapedGlyph, ShapedRun,
    ShapingEngine, ShapingEngineKind, ShapingRequest, TextAttributes, TextFamily,
};
use crate::font_validation::validate_font_file;
use crate::shaping_engines::cosmic_scratch::CosmicScratch;
use crate::{FaceId, TextWeight};

#[derive(Hash, PartialEq, Eq)]
enum FontSourceKey {
    Binary(usize),
    File(PathBuf),
}

/// The cosmic-text-backed [`ShapingEngine`].
///
/// Owns a cosmic-text [`FontSystem`] (locale, font database, shaping caches) plus a registry of
/// known faces. `fontdb::ID` is an opaque slotmap key, so the engine assigns its own sequential
/// registry indexes; those index into both a `fontdb::ID` map (for shaping) and a font-data
/// map (for rasterization through `FaceId`). Fallback faces the shaper selects without an
/// explicit `load_font` call are resolved lazily on first use.
///
/// The engine holds no internal lock: instances are owned by [`crate::FontManager`], whose
/// single outer mutex already serializes all access; locking again here would only add a
/// second, uncontended layer.
pub struct CosmicTextEngine {
    font_system: FontSystem,
    /// The fallback candidate pool (the system-font catalog), prepared once in `system`
    /// and cloned into every scratch seed — never rescanned. Empty for `bare()`.
    candidate_pool: Arc<fontdb::Database>,
    /// One entry per known face, in registration order. The registry index is the [`FaceId`]
    /// payload. The map is published as an immutable `Arc` snapshot after every mutation
    /// (registration and lazy fallback resolution); see [`ShapingEngine::font_registry`].
    faces: Vec<CosmicFace>,
    /// The `faces` snapshot last published as an `Arc`. Rebuilt on every mutation:
    /// resolution is rare after the first use of a fallback face, so the O(n) clone is
    /// acceptable; readers (the manager's published snapshot) stay clone-cheap.
    /// Simplification note: the separate Vec could go if `faces` were itself an Arc, at
    /// the cost of copy-on-write for resolution lookups — kept simple for now.
    published_faces: Arc<Vec<CosmicFace>>,
    /// Faces the shaper resolved without a registry hit, mapped by their `fontdb::ID` so
    /// the (byte-copying) data read and content comparison happens at most once per
    /// distinct face — not per glyph of every cluster, every frame.
    resolved: HashMap<fontdb::ID, FaceId>,
}

impl CosmicTextEngine {
    /// Rebuild `published_faces` from the database after a mutation. Call sites:
    /// `register` and lazy resolution — the only two growth points. `pull` reuses the caller's
    /// published snapshot instead of re-deriving one (see below).
    fn publish(&mut self) {
        self.published_faces = Arc::new(self.faces.clone());
    }

    /// Create a shape-ready clone from the published registry (the cosmic *registry sync*,
    /// ADR 0006), seeded with the candidate pool so fallback selection (emoji, unknown
    /// scripts) can leave the registry's loaded families; an empty pool stays
    /// registry-only. Registry faces are pulled after the pool's, so loaded families win
    /// selection (the white-screen bug was the shaper picking the *system* copy of the
    /// terminal font instead). Built via [`FontSystem::new_with_locale_and_db`] — the
    /// other public constructors rescan system fonts per seed.
    pub(crate) fn seed_from_registry(
        published: &FontRegistry,
        candidate_pool: &Arc<fontdb::Database>,
    ) -> Self {
        let mut engine = Self {
            // Building (not mutating) the FontSystem lets its derived monospace face
            // lists include the pool's faces.
            font_system: Self::seed_font_system(candidate_pool),
            candidate_pool: Arc::clone(candidate_pool),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
            resolved: HashMap::new(),
        };
        engine.pull(published);
        engine
    }

    /// Seed constructor over a clone of the candidate pool (pure in-memory copy — never
    /// rescans); the locale mirrors cosmic-text's std default.
    fn seed_font_system(candidate_pool: &Arc<fontdb::Database>) -> FontSystem {
        // Mirrors cosmic-text's std default (see the module doc for why it is re-declared).
        let locale = "en-US".to_string();
        FontSystem::new_with_locale_and_db(locale, (**candidate_pool).clone())
    }

    /// Incrementally bring this engine in line with the published snapshot: load every
    /// registry entry the engine has not seen yet (payloads are sequential registration-order, so
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
    /// the canonical engine resolves into its own registry (trait `shape`), session-path
    /// clones resolve through the manager — the single `FaceId` authority — so every `FaceId`
    /// issued anywhere is globally valid and the manager's published snapshot carries the
    /// face at registration time.
    pub(crate) fn shape_with_resolver(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        resolve: &mut dyn FnMut(&mut Self, fontdb::ID, fontdb::Weight) -> Option<FaceId>,
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

impl fmt::Debug for CosmicTextEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CosmicTextEngine")
            .field("font_count", &self.faces.len())
            .finish_non_exhaustive()
    }
}

impl CosmicTextEngine {
    /// Create an engine with or without system fonts.
    ///
    /// With them, the one full system-catalog scan for the whole manager family happens here; the
    /// scanned catalog becomes the candidate pool, and scratch seeds clone it (in-memory copy)
    /// instead of rescanning. Without them the pool stays empty and seeds stay registry-only.
    pub fn new(system_fonts: bool) -> Result<Self> {
        let (font_system, candidate_pool) = if system_fonts {
            let db = Self::filtered_system_font_db()?;
            let font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
            let candidate_pool = Arc::new(fontdb::Database::clone(font_system.db()));
            (font_system, candidate_pool)
        } else {
            (
                FontSystem::new_with_locale_and_db("en-US".to_string(), fontdb::Database::new()),
                Arc::default(),
            )
        };

        Ok(Self {
            font_system,
            candidate_pool,
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
            resolved: HashMap::new(),
        })
    }

    /// Load system fonts and retain only sources whose every face Swash can read.
    ///
    /// Faces sharing a source are validated together, so invalid files are filtered as a unit;
    /// an unreadable source fails initialization instead.
    fn filtered_system_font_db() -> Result<fontdb::Database> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        let mut source_faces: HashMap<FontSourceKey, (String, Vec<fontdb::ID>)> = HashMap::new();
        for face in db.faces() {
            let (key, label) = match &face.source {
                Source::Binary(bytes) => (
                    FontSourceKey::Binary(Arc::as_ptr(bytes) as *const () as usize),
                    "binary system font".to_string(),
                ),
                Source::File(path) => (
                    FontSourceKey::File(path.clone()),
                    path.display().to_string(),
                ),
                Source::SharedFile(path, _) => (
                    FontSourceKey::File(path.clone()),
                    path.display().to_string(),
                ),
            };
            source_faces
                .entry(key)
                .or_insert_with(|| (label, Vec::new()))
                .1
                .push(face.id);
        }

        for (_, (label, ids)) in source_faces {
            let first_id = ids[0];
            let Some(validation) =
                db.with_face_data(first_id, |bytes, _| validate_font_file(bytes))
            else {
                log::warn!("Cannot read cosmic-text system font source {label}");
                return Err(anyhow!("cannot read system font source {label}"));
            };
            if let Err(error) = validation {
                log::warn!("Filtering invalid cosmic-text system font source {label}: {error}");
                for id in ids {
                    db.remove_face(id);
                }
            }
        }
        Ok(db)
    }

    /// The candidate pool handed to scratch seeds via `new_scratch` (empty for `bare()`).
    pub(crate) fn candidate_pool(&self) -> &Arc<fontdb::Database> {
        &self.candidate_pool
    }
}

impl ShapingEngine for CosmicTextEngine {
    fn name(&self) -> &'static str {
        "cosmic-text"
    }

    fn load_font(&mut self, data: FontBytes) -> Result<Vec<FaceId>> {
        if let Err(error) = validate_font_file(data.as_ref().as_ref()) {
            log::warn!("Rejecting cosmic-text font file: {error}");
            return Err(error.into());
        }
        let ids = self.register(data);
        if ids.is_empty() {
            return Err(anyhow!("fontdb did not load any faces from the font file"));
        }
        Ok(ids)
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
        Arc::new(FontRegistry::from_owned(fonts))
    }

    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        // Canonical-engine path (see `shape_with_resolver`): unregistered fallback faces
        // resolve into this engine's own registry.
        self.shape_with_resolver(request, font_size, &mut |engine, id, weight| {
            engine.resolve_face_index(id, weight).map(Self::face_id)
        })
    }

    /// Create this engine's per-context scratch (ADR 0006), seeded from the current registry
    /// and a clone of the engine's prepared candidate pool. The full system catalog is never
    /// scanned more than once per manager family (see `system()`); `bare()` keeps the pool
    /// empty, so those shapers remain registry-only.
    fn new_scratch(&self, published: &FontRegistry) -> Box<dyn EngineScratch> {
        Box::new(CosmicScratch::new(
            Arc::clone(self.candidate_pool()),
            published,
        ))
    }

    fn resolve_face(&mut self, data: FontData) -> Option<FaceId> {
        if let Err(error) = validate_font_file(data.data.as_ref().as_ref()) {
            log::warn!(
                "Rejecting lazy cosmic-text fallback font file at face index {}: {error}",
                data.index
            );
            return None;
        }
        // Dedupe by content: session-path shaping resolves fontdb faces (system-scanned
        // duplicates included) whose data may already be registered — e.g. the terminal
        // font loaded via `load_font` matched in the session db as the system-scanned
        // copy of the same family. Resolution would append a duplicate under a *fresh*
        // FaceId every time, and per-frame registry snapshots (read before the session
        // opened) would never carry it — every cluster's metrics lookup would miss and
        // the glyphs would never render. The same font data is always the same `FaceId`.
        // Fast path: the identical
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
        let data_index = data.index;
        self.register(data.data)
            .into_iter()
            .find(|id| self.faces[id.payload() as usize].data_index == data_index)
    }
}

impl CosmicTextEngine {
    /// Shape through this engine's `FontSystem`; faces the shaper selects without a
    /// registered entry are resolved by `resolve` — the canonical engine's trait `shape`
    /// resolves locally, per-clone sessions resolve through the manager (see the ADR 0006
    /// callers of this method).
    fn shape_impl(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        resolve: &mut dyn FnMut(&mut Self, fontdb::ID, fontdb::Weight) -> Option<FaceId>,
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
                    // engine's own registry (canonical sessions) or the manager authority
                    // (per-clone sessions, ADR 0006). Either policy yields a globally
                    // valid id and records the face in the published world.
                    let face_id = match self.lookup(glyph.font_id, glyph.font_weight) {
                        Some(index) => Self::face_id(index),
                        // First sighting of this database face: resolve through `resolve`
                        // (canonical registry or manager resolution), then memoize — the resolver
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

    /// Resolve a shaper-selected (possibly fallback) `fontdb::ID` into this engine's registry
    /// on first use. The face's bytes are read out of the database once and pinned so
    /// rasterization resolves the same data later. Faces the *canonical* engine resolves
    /// land here; per-session engines route through the manager authority instead (ADR 0006).
    /// Its `None` on unreadable face data aborts the run — regression-covered in
    /// `tests.rs` (`cosmic_engine_shapes_through_shared_file_faces`).
    pub(crate) fn resolve_face_index(
        &mut self,
        id: fontdb::ID,
        weight: fontdb::Weight,
    ) -> Option<usize> {
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

    /// Read a database face's raw font data plus its face index (for the manager-resolution
    /// seam, ADR 0006): the same read canonical resolution performs, without growing any
    /// registry. Returns `None` only for unreadable sources (see `resolve_face_index`).
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
