#![allow(dead_code)]

use massive_geometry::{Color, Vector3};
use massive_shapes::{GlyphBrush, GlyphRun, TextWeight, glyph_run_to_run};

/// Convert a single Parley [`parley::GlyphRun`] into a [`massive_shapes::GlyphRun`].
///
/// Delegates to the shared adapter (`massive_shapes::glyph_run_to_run`), which derives
/// ascent/descent/width metrics from the run and normalizes glyph positions to the Y-up convention
/// downstream expects.
pub fn to_glyph_run<'a>(
    translation: Vector3,
    parley_run: parley::GlyphRun<'a, GlyphBrush>,
) -> GlyphRun {
    glyph_run_to_run(
        parley_run,
        Color::BLACK,
        TextWeight::NORMAL,
        translation,
    )
}
