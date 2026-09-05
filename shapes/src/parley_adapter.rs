//! Adapter from Parley layout output to the `GlyphRun` data model.
//!
//! This is the single translation point between Parley and `massive-shapes`. `massive-shapes`
//! owns the [`crate::FontManager`] and derives a [`FontId`] for each concrete font from its
//! `Blob` unique id; this module only knows how to turn a Parley [`Layout`] into [`GlyphRun`]s.
//!
//! Positions are normalized to the **Y-up** convention expected downstream (see the TODO on
//! [`crate::GlyphRun`]): Parley lays out in Y-down, so glyph y offsets are shifted back to the
//! run baseline here.

use parley::{GlyphRun as ParleyGlyphRun, LayoutContext, PositionedLayoutItem, Run};

use massive_geometry::{Color, Vector3};

use crate::{FontId, GlyphRun, GlyphKey, GlyphRunMetrics, RunGlyph, TextWeight};

/// Default Parley brush type (RGBA bytes). Callers overwrite color via [`GlyphRun::with_color`].
pub type GlyphBrush = [u8; 4];

/// Derive the opaque [`FontId`] for a font from its Parley `Blob` unique id.
///
/// The `Blob` id is a distinct atomic counter value, so this needs no registry lookup and matches
/// the id the renderer's rasterization registry is keyed on.
pub fn font_id(font: &parley::FontData) -> FontId {
    FontId(font.data.id())
}

/// Convert a single Parley [`ParleyGlyphRun`] into a [`GlyphRun`].
///
/// `translation` is combined into the returned run; `text_color` and `default_weight` apply when a
/// run does not carry its own values. Metrics and Y-up normalization happen here.
pub fn glyph_run_to_run<'a>(
    parley_run: ParleyGlyphRun<'a, GlyphBrush>,
    text_color: Color,
    default_weight: TextWeight,
    translation: Vector3,
) -> GlyphRun {
    let run = parley_run.run();
    let baseline = parley_run.baseline();

    let weight = TextWeight(run.font_attrs().weight.value() as u16);
    let weight = if weight.0 == 0 { default_weight } else { weight };
    let font_id = font_id(run.font());
    let font_size = run.font_size();

    let glyphs = parley_run
        .positioned_glyphs()
        .map(|glyph| {
            // Parley `Glyph::y` is added to the baseline (Y-down). Shift back to a baseline-relative
            // Y-up coordinate to match the convention `GlyphRun::place_glyph` expects.
            let pos = (glyph.x.round() as i32, (glyph.y - baseline).round() as i32);
            RunGlyph::new(pos, GlyphKey::new(font_id, glyph.id as u16, font_size, weight))
        })
        .collect();

    GlyphRun::new(
        translation,
        glyph_run_metrics(run),
        text_color,
        weight,
        glyphs,
    )
}

fn glyph_run_metrics(run: &Run<'_, GlyphBrush>) -> GlyphRunMetrics {
    let metrics = run.metrics();
    GlyphRunMetrics {
        max_ascent: metrics.ascent.ceil() as u32,
        max_descent: metrics.descent.ceil() as u32,
        width: run.advance().ceil() as u32,
    }
}

/// Create a fresh scratch [`LayoutContext`]. The renderer side may cache this instead for reuse.
pub fn new_layout_context() -> LayoutContext<GlyphBrush> {
    LayoutContext::new()
}

/// Iterate the [`ParleyGlyphRun`]s in a line.
pub fn line_runs<'a>(
    line: &parley::Line<'a, GlyphBrush>,
) -> impl Iterator<Item = ParleyGlyphRun<'a, GlyphBrush>> + 'a {
    line.items().filter_map(|item| match item {
        PositionedLayoutItem::GlyphRun(run) => Some(run),
        PositionedLayoutItem::InlineBox(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextWeight;
    use massive_geometry::{Color, Vector3};
    use parley::FontContext;

    /// A bundled monospace font so the adapter test doesn't depend on system fonts.
    const JETBRAINS_MONO: &[u8] =
        include_bytes!("../../examples/shared/src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");

    /// Shapes a known monospace ASCII string and asserts the produced glyph positions use the
    /// Y-up convention (y ≈ 0 relative to the baseline) and that x positions are monotonic, which
    /// locks the Y-down → Y-up flip performed in [`glyph_run_to_run`].
    #[test]
    fn glyph_run_positions_are_y_up_and_monotonic() {
        let mut font_context = FontContext::new();
        let blob = parley::fontique::Blob::new(std::sync::Arc::new(JETBRAINS_MONO)
            as std::sync::Arc<dyn std::convert::AsRef<[u8]> + Send + Sync>);
        let font_data = parley::FontData::new(blob.clone(), 0);
        font_context.collection.register_fonts(blob, None);
        let font_id = font_id(&font_data);

        let mut layout_context = LayoutContext::new();
        let text = "HI";
        let mut builder = layout_context.ranged_builder(&mut font_context, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(16.0));
        builder.push_default(parley::StyleProperty::FontFamily(parley::FontFamily::named(
            "JetBrains Mono",
        )));
        let mut layout: parley::Layout<GlyphBrush> = builder.build(text);
        layout.break_all_lines(None);
        layout.align(parley::Alignment::Start, Default::default());

        let line = layout.get(0).expect("single line");
        let parley_run = line_runs(&line).next().expect("has a run");
        let run = glyph_run_to_run(
            parley_run,
            Color::WHITE,
            TextWeight::NORMAL,
            Vector3::ZERO,
        );

        assert_eq!(run.glyphs.len(), 2, "two glyphs for two ASCII chars");
        // Y-up: glyph y should be ~0 (baseline-relative), i.e. the same regardless of the run
        // baseline, not offset by the positive Y-down baseline.
        for glyph in &run.glyphs {
            assert!(glyph.pos.1 == 0, "glyph y should be baseline-relative (Y-up), got {:?}", glyph.pos.1);
        }
        // X positions are increasing (monospace, one glyph per cell).
        let xs: Vec<i32> = run.glyphs.iter().map(|g| g.pos.0).collect();
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "x positions must be increasing: {xs:?}");
        // Every glyph resolves to the registered font id.
        for glyph in &run.glyphs {
            assert_eq!(glyph.key.font_id, font_id);
        }
    }
}
