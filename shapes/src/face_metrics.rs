//! Per-face swash metrics, extracted to owned storage and cached on the font manager.
//!
//! Consumers that place shaped glyphs on a grid (the terminal's `cluster_to_run`) read font
//! metrics for every cluster of every frame: a swash `FontRef` construction plus the
//! `glyph_metrics`/`metrics` table parses. swash parses those tables unconditionally, so
//! without a cache the same work repeats per cluster per frame (measured ~7 µs per parse in
//! debug vs ~12 ns cached).
//!
//! [`FaceMetrics`] extracts the values that path needs — glyph left side bearings and
//! units-per-em — into owned storage, once per [`FaceId`]. swash's `GlyphMetrics` borrows the
//! font bytes, so caching the handle itself would pin lifetimes through the manager;
//! extraction is one linear pass over the horizontal-metrics table, negligible next to
//! per-frame re-parses. Fallback faces (emoji etc.) each pay their pass once on first use,
//! exactly like the glyph atlas.
//!
//! The cache lives inside [`crate::FontManager`]'s synchronized state (see
//! [`Shaper::face_metrics`]), so entries are consistent with the fonts the manager's lock
//! protects: an id only ever resolves against the engine that produced it, and the cache
//! cannot outlive the manager instance it belongs to.

use std::collections::HashMap;

use crate::FaceId;
use crate::engine::FontData;

/// The per-face values the glyph placement path reads: left side bearing per glyph id
/// (font units) and the face's units per em.
pub struct FaceMetrics {
    units_per_em: f32,
    /// Left side bearing by glyph id, in font units. Extracted once from the face's
    /// horizontal-metrics data so the per-glyph check is an array read.
    lsb: Vec<f32>,
}

impl FaceMetrics {
    pub fn units_per_em(&self) -> f32 {
        self.units_per_em
    }

    /// The glyph's left side bearing in font units; `0` beyond the table (matching swash,
    /// which clamps out-of-range ids to zero-sized metrics).
    pub fn lsb(&self, glyph_id: u16) -> f32 {
        self.lsb.get(glyph_id as usize).copied().unwrap_or(0.0)
    }
}

/// Cached [`FaceMetrics`] keyed by [`FaceId`]. Owned by [`crate::FontManagerInner`] and
/// accessed through the shaper guard, so no additional synchronization is needed.
#[derive(Default)]
pub struct FaceMetricsCache {
    entries: HashMap<FaceId, FaceMetrics>,
}

impl FaceMetricsCache {
    /// Extract-and-memoize the metrics of the face `font_data` belongs to.
    ///
    /// `font_data` comes from the caller's engine resolution (an `Arc` clone, no bytes
    /// copied). The table pass happens once per face. Called only from the manager's
    /// synchronized state, so the visibility stays module-private.
    pub(crate) fn entry(&mut self, id: FaceId, font_data: FontData) -> Option<&FaceMetrics> {
        if !self.entries.contains_key(&id) {
            let font_ref = swash::FontRef::from_index(
                font_data.data.as_ref().as_ref(),
                font_data.index as usize,
            )?;
            let glyph_metrics = font_ref.glyph_metrics(&[]);
            let lsb: Vec<f32> = (0..glyph_metrics.glyph_count())
                .map(|glyph_id| glyph_metrics.lsb(glyph_id))
                .collect();
            let units_per_em = font_ref.metrics(&[]).units_per_em as f32;
            self.entries
                .entry(id)
                .or_insert(FaceMetrics { units_per_em, lsb });
        }
        self.entries.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use crate::{FontManager, ShapingEngineKind, ShapingRequest, TextAttributes};

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// The cached lsb matches a fresh swash parse for the glyph shaping produces: the
    /// extraction must reproduce swash's semantics (font units, same convention. And the
    /// memoized entry is stable across repeated resolution.
    #[test]
    fn cached_lsb_matches_fresh_swash_parse() {
        for kind in ShapingEngineKind::available() {
            let mut fonts = FontManager::bare(*kind).with_font(JETBRAINS_MONO);
            let face = fonts.load_font(JETBRAINS_MONO)[0];
            let mut shaper = fonts.shaper();
            let shaped = shaper
                .shape(
                    &ShapingRequest::new("a", TextAttributes::named_family("JetBrains Mono")),
                    13.0,
                )
                .expect("shape");
            let glyph_id = shaped.glyphs[0].glyph_id;

            // Resolve font data through the shaper guard: the manager's mutex is already
            // held, so `FontManager::font_data` here would self-deadlock. Copy both values
            // out before the assert so the guard borrow is not held across the comparison.
            let font_data = shaper.font_data(face).expect("font data");
            let cached = shaper
                .face_metrics(face)
                .expect("face metrics")
                .lsb(glyph_id);
            drop(shaper);

            let fresh = swash::FontRef::from_index(
                font_data.data.as_ref().as_ref(),
                font_data.index as usize,
            )
            .expect("fresh font ref")
            .glyph_metrics(&[])
            .lsb(glyph_id);
            assert_eq!(cached, fresh, "engine {kind:?}");

            // Repeated resolution reads the memoized entry with the same values.
            let mut shaper = fonts.shaper();
            let again = shaper
                .face_metrics(face)
                .expect("face metrics")
                .lsb(glyph_id);
            assert_eq!(again, fresh);
        }
    }
}
