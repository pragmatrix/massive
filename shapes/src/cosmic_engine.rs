//! The cosmic-text shaping engine.
//!
//! Shapes through cosmic-text's shaping-only stage (`BufferLine::shape`, the path the
//! pre-Parley terminal code drove — no line breaking, no metrics hinting) and translates the
//! shaped glyphs into the engine-neutral model. `ShapeGlyph` advances/offsets/ascent are
//! em-relative (cosmic-text divides by the font scale at shape time), so the engine multiplies
//! by the requested `font_size` to get pixels.

use std::sync::{Arc, Mutex};

use cosmic_text::{Attrs, AttrsList, BufferLine, FontSystem, LineEnding, Shaping, Weight};
use fontdb::Source;

use crate::engine::{
    FontBytes, FontData, ShapedCluster, ShapedGlyph, ShapedRun, ShapingEngine, ShapingRequest,
    TextAttributes, TextFamily,
};
use crate::{FaceId, TextWeight};

/// Engine tag for [`FaceId`] packing (see `FaceId::from_packed`).
const ENGINE_TAG: u64 = 2;

/// The cosmic-text-backed [`ShapingEngine`].
///
/// Owns a cosmic-text [`FontSystem`] (locale, font database, shaping caches) plus a registry of
/// known faces. `fontdb::ID` is an opaque slotmap key, so the engine assigns its own sequential
/// registry indexes; those index into both a `fontdb::ID` map (for shaping) and a font-data
/// map (for rasterization through `FaceId`). Fallback faces the shaper selects without an
/// explicit `load_font` call are interned lazily on first use.
#[derive(Clone)]
pub struct CosmicTextEngine(Arc<Mutex<CosmicTextEngineInner>>);

struct CosmicTextEngineInner {
    font_system: FontSystem,
    /// One entry per known face, in registration order. The registry index is the [`FaceId`]
    /// payload.
    faces: Vec<CosmicFace>,
}

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
        let inner = self.0.lock().unwrap();
        f.debug_struct("CosmicTextEngine")
            .field("font_count", &inner.faces.len())
            .finish_non_exhaustive()
    }
}

impl CosmicTextEngine {
    /// Create a completely bare engine: no fallbacks, no fonts.
    pub fn bare() -> Self {
        Self(Arc::new(Mutex::new(CosmicTextEngineInner {
            font_system: FontSystem::new_with_fonts(core::iter::empty()),
            faces: Vec::new(),
        })))
    }

    /// Create an engine with the environment's locale, system fonts, and fallbacks loaded.
    pub fn system() -> Self {
        Self(Arc::new(Mutex::new(CosmicTextEngineInner {
            font_system: FontSystem::new(),
            faces: Vec::new(),
        })))
    }

    fn face_id(index: usize) -> FaceId {
        FaceId::from_packed(ENGINE_TAG, index as u64)
    }

    /// Register a font file's faces into the database and the registry.
    fn register(inner: &mut CosmicTextEngineInner, data: FontBytes) -> Vec<FaceId> {
        // fontdb holds its own shared reference to the bytes.
        let source = Source::Binary(Arc::clone(&data) as Arc<dyn AsRef<[u8]> + Send + Sync>);
        let ids = inner.font_system.db_mut().load_font_source(source);
        ids.into_iter()
            .map(|id| {
                let face_info = inner.font_system.db().face(id).expect("just-loaded face");
                let index = inner.faces.len();
                inner.faces.push(CosmicFace {
                    id,
                    weight: face_info.weight,
                    data: Arc::clone(&data),
                    data_index: face_info.index,
                });
                Self::face_id(index)
            })
            .collect()
    }

    /// Intern a shaper-selected (possibly fallback) `fontdb::ID` into the registry on first use.
    ///
    /// The face's bytes are read out of the database once and pinned in the registry, so
    /// rasterization resolves the same data later.
    fn intern(
        inner: &mut CosmicTextEngineInner,
        id: fontdb::ID,
        weight: fontdb::Weight,
    ) -> Option<usize> {
        if let Some(index) = inner
            .faces
            .iter()
            .position(|face| face.id == id && face.weight == weight)
        {
            return Some(index);
        }
        // Robustness: `SharedFile` sources would need mmap'd data we cannot hand out as an Arc;
        // system fontdb sources on Linux/WASM can be files. We only intern what we can read.
        let (data, data_index) = inner
            .font_system
            .db()
            .with_face_data(id, |bytes, face_index| {
                (Arc::new(bytes.to_vec()) as FontBytes, face_index)
            })?;
        let index = inner.faces.len();
        inner.faces.push(CosmicFace {
            id,
            weight,
            data,
            data_index,
        });
        Some(index)
    }

    fn shape_with_font_system(
        inner: &mut CosmicTextEngineInner,
        request: &ShapingRequest<'_>,
        font_size: f32,
    ) -> Option<ShapedRun> {
        // The shaping-only path: no line breaking, no metrics hinting.
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
        let shape_line = buffer.shape(&mut inner.font_system, 0);

        let mut max_ascent = 0.0_f32;
        let mut max_descent = 0.0_f32;
        let mut width = 0.0_f32;
        let mut clusters: Vec<ShapedCluster> = Vec::with_capacity(request.text.len());

        for span in &shape_line.spans {
            for word in &span.words {
                // A word's glyphs share one line position; the cluster origin is the line
                // advance so far (plus the glyph's own x offset) and glyph x stays intra-cluster
                // so the `GlyphRun` left-bearing logic works engine-independently.
                for glyph in &word.glyphs {
                    // ShapeGlyph units are em-relative; scale to pixels.
                    let glyph_px = font_size * glyph.x_advance;
                    let offset_px = font_size * glyph.x_offset;
                    let y_px = font_size * glyph.y_offset;

                    let index = Self::intern(inner, glyph.font_id, glyph.font_weight)?;
                    let face_id = Self::face_id(index);

                    let cluster_x = width + offset_px;
                    max_ascent = max_ascent.max(font_size * glyph.ascent);
                    max_descent = max_descent.max(font_size * glyph.descent);
                    width += glyph_px;

                    clusters.push(ShapedCluster {
                        byte_range: glyph.start..glyph.end,
                        x: cluster_x,
                        glyphs: vec![ShapedGlyph {
                            glyph_id: glyph.glyph_id,
                            face_id,
                            font_size,
                            weight: TextWeight(glyph.font_weight.0),
                            // Intra-cluster: this engine emits one glyph per cluster, so the
                            // glyph's own offset is 0 relative to the cluster origin.
                            x: 0.0,
                            y: y_px,
                        }],
                    });
                }
            }
        }

        Some(ShapedRun {
            clusters,
            max_ascent,
            max_descent,
            width,
        })
    }
}

/// Build cosmic-text attrs for one attributes value.
///
/// The returned `Attrs` borrows the family name for as long as `attributes` is borrowed; the
/// caller shapes within that lifetime (`BufferLine::new` copies the text but the attr list keeps
/// the family borrow until shaping finishes).
fn attrs<'a>(attributes: &'a TextAttributes<'_>) -> Attrs<'a> {
    let base = Attrs::new().weight(Weight(attributes.weight.0));
    match &attributes.family {
        TextFamily::Named(name) => base.family(cosmic_text::Family::Name(name.as_ref())),
        TextFamily::SansSerif => base.family(cosmic_text::Family::SansSerif),
        TextFamily::Serif => base.family(cosmic_text::Family::Serif),
        TextFamily::Monospace => base.family(cosmic_text::Family::Monospace),
    }
}

impl ShapingEngine for CosmicTextEngine {
    fn name(&self) -> &'static str {
        "cosmic-text"
    }

    fn load_font(&mut self, data: FontBytes) -> Vec<FaceId> {
        let mut inner = self.0.lock().unwrap();
        Self::register(&mut inner, data)
    }

    fn font_data(&self, id: FaceId) -> Option<FontData> {
        debug_assert_eq!(id.engine_tag(), ENGINE_TAG, "foreign FaceId");
        let inner = self.0.lock().unwrap();
        let face = inner.faces.get(id.payload() as usize)?;
        Some(FontData {
            data: Arc::clone(&face.data),
            index: face.data_index,
        })
    }

    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        let mut inner = self.0.lock().unwrap();
        Self::shape_with_font_system(&mut inner, request, font_size)
    }
}
