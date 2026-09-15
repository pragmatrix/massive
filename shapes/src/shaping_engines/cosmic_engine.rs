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
}

impl CosmicTextEngine {
    /// Rebuild `published_faces` from `self.faces` after a mutation. Call sites:
    /// `register` and `intern` — the only two growth points.
    fn publish(&mut self) {
        self.published_faces = Arc::new(self.faces.clone());
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
            font_system: FontSystem::new_with_fonts(core::iter::empty()),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
        }
    }

    /// Create an engine with the environment's locale, system fonts, and fallbacks loaded.
    pub fn system() -> Self {
        Self {
            font_system: FontSystem::new(),
            faces: Vec::new(),
            published_faces: Arc::new(Vec::new()),
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
        Arc::new(FontRegistry::new(Arc::new(
            self.published_faces
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
                .collect::<HashMap<_, _>>(),
        )))
    }

    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        self.shape_with_font_system(request, font_size)
    }
}

impl CosmicTextEngine {
    fn shape_with_font_system(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
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

                    let index = self.intern(glyph.font_id, glyph.font_weight)?;
                    let face_id = Self::face_id(index);

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

    /// Intern a shaper-selected (possibly fallback) `fontdb::ID` into the registry on first use.
    ///
    /// The face's bytes are read out of the database once and pinned in the registry, so
    /// rasterization resolves the same data later.
    fn intern(&mut self, id: fontdb::ID, weight: fontdb::Weight) -> Option<usize> {
        if let Some(index) = self
            .faces
            .iter()
            .position(|face| face.id == id && face.weight == weight)
        {
            return Some(index);
        }
        // Robustness: `SharedFile` sources hand out their bytes as an Arc we can read here;
        // only an unreadable `Source::File` (deleted/unreadable path) would return None.
        let (data, data_index) = self
            .font_system
            .db()
            .with_face_data(id, |bytes, face_index| {
                (Arc::new(bytes.to_vec()) as FontBytes, face_index)
            })?;
        let index = self.faces.len();
        self.faces.push(CosmicFace {
            id,
            weight,
            data,
            data_index,
        });
        self.publish();
        Some(index)
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
