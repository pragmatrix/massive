//! Bridge helpers for the `markdown`/`emojis` example binaries.
//!
//! These examples use the vendored `inlyne` crate for layout/positioning (which needs its own
//! cosmic-text `FontSystem` for measuring). Cosmic-text remains a dependency here only for that
//! layout pipeline; the final glyphs are converted to [`massive_shapes::GlyphRun`] so the renderer
//! data path stays on the engine-neutral text pipeline (ADR 0005).
//!
//! [`FontBridge`] converts cosmic-text's own glyph coordinates into [`GlyphRun`]s, mapping each
//! cosmic-text `fontdb::ID` to the [`FaceId`] of the same face in the manager's engine. This
//! avoids re-shaping the text (which could diverge from cosmic-text's layout) and keeps the
//! renderer's rasterization path on the engine's font registry.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::fontdb;
use swash::FontRef;

use massive_geometry::{Color, Vector3};
use massive_shapes::ClipBoxPx;
use massive_shapes::ShapingEngineKind;
use massive_shapes::{FaceId, FontData, GlyphKey, GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

/// Bridges cosmic-text's font database to the shaping engine behind a [`massive_shapes::FontManager`],
/// so cosmic-text glyphs can be converted to [`GlyphRun`]s that rasterize through that engine.
///
/// Owns the manager and the cosmic-text `fontdb::Database`, plus a map from each
/// `fontdb::ID` to the engine [`FaceId`] for the same face. The map is built at construction time
/// by registering the same font bytes into both databases and pairing faces by index.
pub struct FontBridge {
    font_manager: massive_shapes::FontManager,
    font_db: fontdb::Database,
    /// Maps a cosmic-text `fontdb::ID` to the engine [`FaceId`] for the same face.
    face_ids: HashMap<fontdb::ID, FaceId>,
}

impl FontBridge {
    /// Build a bridge over the system fonts, registering every system face into both databases.
    ///
    /// Enumerates the system fonts from a fresh `fontdb::Database`, registers each face's bytes
    /// into a bare engine manager, and pairs each `fontdb::ID` with the engine [`FaceId`]
    /// for the same face. This keeps the two databases in sync so any font cosmic-text selects
    /// (including emoji fallbacks) resolves to a [`FaceId`] for rasterization.
    ///
    /// This eagerly reads every system font file and retains its bytes, which can consume a large
    /// amount of memory. It is suitable for this example's complete fallback coverage, but not
    /// for production startup; production code should load only fonts selected by shaping.
    pub fn system() -> Self {
        let mut font_db = fontdb::Database::new();
        font_db.load_system_fonts();
        // Cosmic-text matches inlyne's FontSystem, which does the measuring for these examples.
        let font_manager = massive_shapes::FontManager::bare(ShapingEngineKind::CosmicText);

        // Register each unique system font file once and map every fontdb face back to the
        // corresponding engine FaceId. A collection can expose several faces and family names,
        // while one batch publication avoids rebuilding the growing registry for each file.
        let mut file_indices = HashMap::new();
        let mut font_files = Vec::new();
        for face in font_db.faces() {
            let path = match &face.source {
                fontdb::Source::File(path) => path,
                fontdb::Source::Binary(_) | fontdb::Source::SharedFile(_, _) => continue,
            };
            if file_indices.contains_key(path) {
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            file_indices.insert(path.clone(), font_files.len());
            font_files.push(bytes);
        }

        let loaded_files = font_manager.load_fonts(font_files);
        let mut face_ids = HashMap::new();
        for face in font_db.faces() {
            let path = match &face.source {
                fontdb::Source::File(path) => path,
                fontdb::Source::Binary(_) | fontdb::Source::SharedFile(_, _) => continue,
            };
            let Some(engine_ids) = file_indices
                .get(path)
                .and_then(|file_index| loaded_files.get(*file_index))
            else {
                continue;
            };
            // load_font returns one id per face in file order; pick the face at `index`.
            if let Some(face_id) = engine_ids.get(face.index as usize).copied() {
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
        let engine_ids = font_manager.load_font(font_bytes.clone());
        // fontdb's `Source::Binary` needs a trait-object Arc; clone the bytes into a `Vec` so the
        // two databases each hold their own reference to the same data.
        let fontdb_source = fontdb::Source::Binary(
            Arc::new(font_bytes.to_vec()) as Arc<dyn AsRef<[u8]> + Send + Sync>
        );
        let fontdb_ids = font_db.load_font_source(fontdb_source);
        let mut face_ids = HashMap::new();
        for fontdb_id in fontdb_ids {
            let index = font_db.face(fontdb_id).map(|f| f.index).unwrap_or(0);
            if let Some(face_id) = engine_ids.get(index as usize).copied() {
                face_ids.insert(fontdb_id, face_id);
            }
        }
        Self {
            font_manager,
            font_db,
            face_ids,
        }
    }

    /// The font manager, for the renderer's rasterization path.
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
            self.font_manager.engine_kind(),
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
            .published()
            .font_data(face_id)
            .and_then(|font| font_metrics(&font, font_size))
            .unwrap_or((0.0, 0.0));
        GlyphRunMetrics::from_float(ascent, descent, run.line_w)
    }
}

/// Pixel ascent/descent of a font at `font_size`, from its swash metrics.
fn font_metrics(font: &FontData, font_size: f32) -> Option<(f32, f32)> {
    let font_ref = FontRef::from_index(font.data.as_ref().as_ref(), font.index as usize)?;
    let metrics = font_ref.metrics(&[]);
    let units = metrics.units_per_em as f32;
    Some((
        metrics.ascent * font_size / units,
        metrics.descent * font_size / units,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::process::Command;

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
            massive_shapes::FontManager::bare(ShapingEngineKind::CosmicText),
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

    /// Reports the memory cost of eagerly registering every system font.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "manual memory measurement"]
    fn measure_system_font_memory() {
        let before_rss_kib = process_rss_kib();
        let bridge = FontBridge::system();
        let after_rss_kib = process_rss_kib();

        let mut paths = HashSet::new();
        let mut file_bytes = 0u64;
        for face in bridge.font_db().faces() {
            let fontdb::Source::File(path) = &face.source else {
                continue;
            };
            if paths.insert(path) {
                file_bytes += std::fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
            }
        }

        let rss_delta = match (before_rss_kib, after_rss_kib) {
            (Some(before), Some(after)) => {
                format!("{} KiB RSS delta", after.saturating_sub(before))
            }
            _ => "RSS unavailable".to_owned(),
        };

        println!(
            "system font memory: {} unique files, {} faces, {:.1} MiB font files, {}",
            paths.len(),
            bridge.font_db().faces().count(),
            file_bytes as f64 / (1024.0 * 1024.0),
            rss_delta,
        );
    }

    #[cfg(target_os = "macos")]
    fn process_rss_kib() -> Option<u64> {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p"])
            .arg(std::process::id().to_string())
            .output()
            .ok()?;
        let rss = String::from_utf8(output.stdout).ok()?;
        rss.trim().parse().ok()
    }
}
