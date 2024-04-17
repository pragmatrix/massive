use cosmic_text::{CacheKey, CacheKeyFlags, LayoutGlyph, LayoutRun};
use massive_geometry::Color;
use massive_shapes::{GlyphRun, GlyphRunMetrics, PositionedGlyph};

const RENDER_SUBPIXEL: bool = false;

/// Converts a cosmic_text `LayoutRun` into a `GlyphRun`.
pub fn to_glyph_run(run: &LayoutRun, line_height: f32) -> GlyphRun {
    let max_ascent = run.line_y - run.line_top;

    let glyph_run_metrics = GlyphRunMetrics {
        max_ascent: max_ascent.ceil() as _,
        max_descent: (line_height - max_ascent).ceil() as _,
        width: run.line_w.ceil() as u32,
    };

    let positioned = position_glyphs(run.glyphs);
    GlyphRun::new(glyph_run_metrics, Color::BLACK, positioned)
}

/// Position individual `LayoutGlyph` from a `LayoutRun`.
pub fn position_glyphs(glyphs: &[LayoutGlyph]) -> Vec<PositionedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            let fractional_pos = if RENDER_SUBPIXEL {
                (glyph.x, glyph.y)
            } else {
                (glyph.x.round(), glyph.y.round())
            };

            let (ck, x, y) = CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
                CacheKeyFlags::empty(),
            );
            // Note: hitbox with is fractional, but does not change with / without subpixel
            // rendering.
            PositionedGlyph::new(ck, (x, y), glyph.w)
        })
        .collect()
}
