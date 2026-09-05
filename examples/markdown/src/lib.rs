//! Bridge helpers for the `markdown`/`emojis` example binaries.
//!
//! These examples use the vendored `inlyne` crate for layout/positioning (which needs its own
//! cosmic-text `FontSystem` for measuring). Cosmic-text remains a dependency here only for that
//! layout pipeline; the final glyphs are converted to [`massive_shapes::GlyphRun`] through Parley
//! so the renderer data path stays on the Parley-based text pipeline.

use massive_geometry::{Color, Vector3};
use massive_shapes::{GlyphBrush, GlyphRun, TextWeight};
use massive_shell::FontManager;

/// Re-shape the visible glyph segment of a cosmic-text [`cosmic_text::LayoutRun`] through Parley.
///
/// Cosmic-text already wrapped the line; we carve the run's covered text span (from its first to
/// last glyph), re-shape it as a single Parley segment with no line-breaking, and convert it to a
/// [`GlyphRun`] positioned at `(left, top + run.line_top)`.
pub fn cosmic_run_to_glyph_run(
    fonts: &FontManager,
    run: &cosmic_text::LayoutRun<'_>,
    left: f32,
    top: f32,
) -> Option<GlyphRun> {
    let (first, last) = (run.glyphs.first()?, run.glyphs.last()?);
    let seg_start = first.start;
    let seg_end = last.end;
    let text = &run.text[seg_start..seg_end];
    if text.trim().is_empty() {
        return None;
    }

    let metrics = run.glyphs.first().map(|g| g.font_size).unwrap_or(16.0);

    let translation = Vector3::new(left as f64, (top + run.line_top) as f64, 0.0);

    fonts.with_shape(|fcx, lcx, resolver| {
        let mut builder = lcx.ranged_builder(fcx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontSize(metrics));
        let mut layout: parley::Layout<GlyphBrush> = builder.build(text);
        layout.break_all_lines(None);
        layout.align(parley::Alignment::Start, Default::default());
        let line = layout.get(0)?;
        let parley_run = massive_shapes::line_runs(&line).next()?;
        Some(massive_shapes::glyph_run_to_run(
            parley_run,
            resolver,
            Color::BLACK,
            TextWeight::NORMAL,
            translation,
        ))
    })
}

/// Re-shape all visible segments of a cosmic-text buffer through Parley.
pub fn cosmic_buffer_to_glyph_runs(
    fonts: &FontManager,
    buffer: &cosmic_text::Buffer,
    left: f32,
    top: f32,
) -> Vec<GlyphRun> {
    buffer
        .layout_runs()
        .filter_map(|run| cosmic_run_to_glyph_run(fonts, &run, left, top))
        .collect()
}

