//! Per-face swash metrics, published alongside the font registry (ADR 0006).
//!
//! Consumers that place shaped glyphs on a grid (the terminal's `cluster_to_run`) read font
//! metrics for every cluster of every frame: a swash `FontRef` construction plus the
//! `glyph_metrics`/`metrics` table parses. swash parses those tables unconditionally, so
//! without pre-extraction the same work repeats per cluster per frame (measured ~7 µs per
//! parse in debug vs ~12 ns for an extracted-array read).
//!
//! [`FaceMetrics`] extracts the values that path needs — glyph left side bearings and
//! units-per-em — into owned storage. Extraction happens **eagerly at face-mint time**
//! (`load_font`, lazy fallback interning), under the manager lock that minting already
//! holds, and the result is published with the registry snapshot: readers resolve metrics
//! lock-free through `FontManager::published`, exactly like the rasterizer resolves font
//! data. Values are instance-independent because faces are content-identical.

use swash::FontRef;

use crate::FaceId;
use crate::engine::FontData;

/// The per-face values the glyph placement path reads: left side bearing per glyph id
/// (font units) and the face's units per em.
#[derive(Clone)]
pub struct FaceMetrics {
    units_per_em: f32,
    /// Left side bearing by glyph id, in font units. Extracted once from the face's
    /// horizontal-metrics data so the per-glyph check is an array read.
    lsb: Vec<f32>,
}

impl FaceMetrics {
    /// Extract the metrics of a face from its font data — one linear pass over the
    /// horizontal-metrics table, performed once per face at mint time (see module doc).
    pub fn extract(font_data: &FontData) -> Option<Self> {
        let font_ref =
            FontRef::from_index(font_data.data.as_ref().as_ref(), font_data.index as usize)?;
        let glyph_metrics = font_ref.glyph_metrics(&[]);
        let lsb: Vec<f32> = (0..glyph_metrics.glyph_count())
            .map(|glyph_id| glyph_metrics.lsb(glyph_id))
            .collect();
        let units_per_em = font_ref.metrics(&[]).units_per_em as f32;
        Some(Self { units_per_em, lsb })
    }

    pub fn units_per_em(&self) -> f32 {
        self.units_per_em
    }

    /// The glyph's left side bearing in font units; `0` beyond the table (matching swash,
    /// which clamps out-of-range ids to zero-sized metrics).
    pub fn lsb(&self, glyph_id: u16) -> f32 {
        self.lsb.get(glyph_id as usize).copied().unwrap_or(0.0)
    }
}

/// Extract metrics for every face of a registry-entry map (mint-time; see module doc).
pub(crate) fn extract_all(
    entries: &std::collections::HashMap<FaceId, FontData>,
) -> std::collections::HashMap<FaceId, FaceMetrics> {
    entries
        .iter()
        .filter_map(|(id, data)| Some((*id, FaceMetrics::extract(data)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{FontManager, ShapingEngineKind, ShapingRequest, TextAttributes};

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// The published-metrics lsb matches a fresh swash parse for the glyph shaping
    /// produces: the extraction must reproduce swash's semantics (font units, same
    /// convention).
    #[test]
    fn published_lsb_matches_fresh_swash_parse() {
        for kind in ShapingEngineKind::available() {
            let fonts = FontManager::bare(*kind).with_font(JETBRAINS_MONO);
            let face = fonts.load_font(JETBRAINS_MONO)[0];
            let mut shaper = fonts.session();
            let shaped = shaper
                .shape(
                    &ShapingRequest::new("a", TextAttributes::named_family("JetBrains Mono")),
                    13.0,
                )
                .expect("shape");
            let glyph_id = shaped.glyphs[0].glyph_id;

            // Resolve font data through the session guard: the manager's mutex-protected
            // registry is not lock-free-readable while the session may mint. Copy both
            // values out before the assert so the guard borrow is not held across it.
            let font_data = shaper.font_data(face).expect("font data");
            drop(shaper);

            // Read the published snapshot after the session republished it.
            let cached = fonts
                .published()
                .metrics(face)
                .expect("published metrics must carry every minted face")
                .lsb(glyph_id);

            let fresh = swash::FontRef::from_index(
                font_data.data.as_ref().as_ref(),
                font_data.index as usize,
            )
            .expect("fresh font ref")
            .glyph_metrics(&[])
            .lsb(glyph_id);
            assert_eq!(cached, fresh, "engine {kind:?}");
        }
    }
}
