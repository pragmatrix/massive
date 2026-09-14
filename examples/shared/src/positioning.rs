//! Positioning helpers retained for example API compatibility.

#![allow(dead_code)]

use massive_geometry::Vector3;

use massive_shapes::{GlyphRun, ShapedRun, TextWeight, shaped_run_to_glyph_run};

/// Assemble a [`GlyphRun`] from an engine-neutral shaped run.
///
/// Replaces the former Parley-specific adapter; shaping engines now produce [`ShapedRun`]s
/// directly (see `massive_shapes::ShapingEngine`).
pub fn to_glyph_run(translation: Vector3, run: &ShapedRun) -> GlyphRun {
    shaped_run_to_glyph_run(
        run,
        massive_geometry::Color::BLACK,
        TextWeight::NORMAL,
        translation,
    )
}
