//! Bridge helpers for the `markdown`/`emojis` example binaries.
//!
//! These examples use the vendored `inlyne` crate for layout/positioning (which needs its own
//! cosmic-text `FontSystem` for measuring). Cosmic-text remains a dependency here only for that
//! layout pipeline; the final glyphs are converted to [`massive_shapes::GlyphRun`] so the renderer
//! data path stays on the Parley-based text pipeline.
//!
//! [`FontBridge`] converts cosmic-text's own glyph coordinates into [`GlyphRun`]s, mapping each
//! cosmic-text `fontdb::ID` to the Parley [`FaceId`] for the same face. This avoids re-shaping the
//! text through Parley (which could diverge from cosmic-text's layout) and keeps the renderer's
//! rasterization path on Parley's font registry.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cosmic_text::fontdb;
use parley::FontData;
use swash::FontRef;

use massive_geometry::{Color, Vector3};
use massive_shapes::ClipBoxPx;
use massive_shapes::{FaceId, GlyphKey, GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

/// Bridges cosmic-text's font database to Parley's, so cosmic-text glyphs can be converted to
/// [`GlyphRun`]s that rasterize through Parley.
///
/// Owns the Parley [`FontManager`] and the cosmic-text `fontdb::Database`, plus a map from each
/// `fontdb::ID` to the Parley [`FaceId`] for the same face. The map is built at construction time
/// by registering the same font bytes into both databases and pairing faces by index.
pub struct FontBridge {
    font_manager: massive_shapes::FontManager,
    font_db: fontdb::Database,
    /// Maps a cosmic-text `fontdb::ID` to the Parley [`FaceId`] for the same face.
    face_ids: HashMap<fontdb::ID, FaceId>,
}

impl FontBridge {
    /// Build a bridge over the system fonts, registering every system face into both databases.
    ///
    /// Enumerates the system fonts from a fresh `fontdb::Database`, registers each face's bytes
    /// into a bare Parley [`FontManager`], and pairs each `fontdb::ID` with the Parley [`FaceId`]
    /// for the same face. This keeps the two databases in sync so any font cosmic-text selects
    /// (including emoji fallbacks) resolves to a Parley [`FaceId`] for rasterization.
    pub fn system() -> Self {
        let mut font_db = fontdb::Database::new();
        font_db.load_system_fonts();
        let font_manager = massive_shapes::FontManager::bare();

        // Register each system face into Parley and record the fontdb::ID -> FaceId pairing.
        // Deduplicate by (path, index) so a face shared across families is registered once.
        let mut face_ids = HashMap::new();
        let mut seen = HashSet::new();
        for face in font_db.faces() {
            let (path, index) = match &face.source {
                fontdb::Source::File(path) => (path.clone(), face.index),
                fontdb::Source::Binary(_) | fontdb::Source::SharedFile(_, _) => continue,
            };
            if !seen.insert((path.clone(), index)) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let parley_ids = font_manager.load_font(bytes);
            if let Some(face_id) = parley_ids.iter().find(|id| id.index() == index).copied() {
                face_ids.insert(face.id, face_id);
            }
        }

        Self {
            font_manager,
            font_db,
            face_ids,
        }
    }

    /// Register `font_bytes` into both databases and build the `fontdb::ID → FaceId` map.
    ///
    /// A single font file may hold several faces (e.g. a `.ttc`); each is paired by face index so
    /// the map stays correct for collections.
    pub fn new(
        font_manager: massive_shapes::FontManager,
        mut font_db: fontdb::Database,
        font_bytes: Arc<[u8]>,
    ) -> Self {
        let parley_ids = font_manager.load_font(font_bytes.clone());
        // fontdb's `Source::Binary` needs a trait-object Arc; clone the bytes into a `Vec` so the
        // two databases each hold their own reference to the same data.
        let fontdb_source = fontdb::Source::Binary(
            Arc::new(font_bytes.to_vec()) as Arc<dyn AsRef<[u8]> + Send + Sync>
        );
        let fontdb_ids = font_db.load_font_source(fontdb_source);
        let mut face_ids = HashMap::new();
        for fontdb_id in fontdb_ids {
            let index = font_db.face(fontdb_id).map(|f| f.index).unwrap_or(0);
            if let Some(face_id) = parley_ids.iter().find(|id| id.index() == index).copied() {
                face_ids.insert(fontdb_id, face_id);
            }
        }
        Self {
            font_manager,
            font_db,
            face_ids,
        }
    }

    /// The Parley font manager, for the renderer's rasterization path.
    pub fn font_manager(&self) -> &massive_shapes::FontManager {
        &self.font_manager
    }

    /// The cosmic-text font database, for building a `FontSystem`.
    pub fn font_db(&self) -> &fontdb::Database {
        &self.font_db
    }

    /// Convert every visible run of a cosmic-text buffer into [`GlyphRun`]s.
    pub fn cosmic_buffer_to_glyph_runs(
        &self,
        buffer: &cosmic_text::Buffer,
        left: f32,
        top: f32,
    ) -> Vec<GlyphRun> {
        buffer
            .layout_runs()
            .filter_map(|run| self.cosmic_run_to_glyph_run(&run, left, top))
            .collect()
    }

    /// Convert a single cosmic-text [`cosmic_text::LayoutRun`] into a [`GlyphRun`].
    ///
    /// Uses cosmic-text's own glyph coordinates (line-relative x, baseline-relative y), matching
    /// the [`GlyphRun`] contract the Parley adapter produces: `pos.y` is baseline-relative and the
    /// renderer adds `max_ascent` to position the glyph box. The run is positioned at
    /// `(left, top + run.line_top)`.
    pub fn cosmic_run_to_glyph_run(
        &self,
        run: &cosmic_text::LayoutRun<'_>,
        left: f32,
        top: f32,
    ) -> Option<GlyphRun> {
        let first = run.glyphs.first()?;
        let face_id = self.face_ids.get(&first.font_id).copied()?;
        let font_size = first.font_size;
        let weight = TextWeight(first.font_weight.0);

        let translation = Vector3::new(left as f64, (top + run.line_top) as f64, 0.0);

        let glyphs = run
            .glyphs
            .iter()
            .map(|glyph| {
                let face_id = self
                    .face_ids
                    .get(&glyph.font_id)
                    .copied()
                    .unwrap_or(face_id);
                RunGlyph::new(
                    (glyph.x.round() as i32, glyph.y.round() as i32),
                    GlyphKey::new(
                        face_id,
                        glyph.glyph_id,
                        glyph.font_size,
                        TextWeight(glyph.font_weight.0),
                        ClipBoxPx::UNCLIPPED,
                    ),
                )
            })
            .collect();

        let metrics = self.run_metrics(run, face_id, font_size);

        Some(GlyphRun::new(
            translation,
            metrics,
            Color::BLACK,
            weight,
            glyphs,
        ))
    }

    /// Compute [`GlyphRunMetrics`] for a run from the first glyph's font metrics.
    fn run_metrics(
        &self,
        run: &cosmic_text::LayoutRun<'_>,
        face_id: FaceId,
        font_size: f32,
    ) -> GlyphRunMetrics {
        let (ascent, descent) = self
            .font_manager
            .font_data(face_id)
            .and_then(|font| font_metrics(&font, font_size))
            .unwrap_or((0.0, 0.0));
        GlyphRunMetrics::from_float(ascent, descent, run.line_w)
    }
}

/// Pixel ascent/descent of a font at `font_size`, from its swash metrics.
fn font_metrics(font: &FontData, font_size: f32) -> Option<(f32, f32)> {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index as usize)?;
    let metrics = font_ref.metrics(&[]);
    let units = metrics.units_per_em as f32;
    Some((
        metrics.ascent * font_size / units,
        metrics.descent * font_size / units,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::{Align, Attrs, Buffer, FontSystem, Metrics, Shaping};

    /// A bundled monospace font so the test doesn't depend on system fonts.
    const MONTSERRAT: &[u8] =
        include_bytes!("../../../assets/fonts/Montserrat/Montserrat-Regular.ttf");

    /// Shapes a known string through cosmic-text and asserts the bridge emits baseline-relative
    /// Y-up glyph positions (y ≈ 0) and monotonic x, locking the coordinate conversion.
    #[test]
    fn cosmic_run_positions_are_baseline_relative_and_monotonic() {
        let bridge = FontBridge::new(
            massive_shapes::FontManager::bare(),
            fontdb::Database::new(),
            Arc::from(MONTSERRAT),
        );
        let mut font_system =
            FontSystem::new_with_locale_and_db("en-US".into(), bridge.font_db().clone());
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 20.0));
        buffer.set_text("HI", &Attrs::new(), Shaping::Advanced, Some(Align::Left));
        buffer.shape_until_scroll(&mut font_system, false);

        let runs = bridge.cosmic_buffer_to_glyph_runs(&buffer, 0.0, 0.0);
        let run = runs.first().expect("has a run");
        assert!(
            run.glyphs.len() >= 2,
            "two glyphs for two ASCII chars, got {}",
            run.glyphs.len()
        );
        for glyph in &run.glyphs {
            assert!(
                glyph.pos.y == 0,
                "glyph y should be baseline-relative (Y-up), got {:?}",
                glyph.pos.y
            );
        }
        let xs: Vec<i32> = run.glyphs.iter().map(|g| g.pos.x).collect();
        assert!(
            xs.windows(2).all(|w| w[0] < w[1]),
            "glyph x should be monotonic, got {:?}",
            xs
        );
    }
}
