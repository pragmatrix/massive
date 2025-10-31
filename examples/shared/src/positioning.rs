#![allow(dead_code)]

use cosmic_text::{CacheKey, CacheKeyFlags, LayoutGlyph, LayoutRun};

use itertools::Itertools;
use massive_geometry::{Color, Vector3};
use massive_shapes::{GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

const RENDER_SUBPIXEL: bool = false;

/// Converts a cosmic_text `LayoutRun` into one or more `GlyphRun`s.
///
/// We split `LayoutRun`s if they contain different metadata which points to a color.
pub fn to_attributed_glyph_runs(
    translation: Vector3,
    run: &LayoutRun,
    line_height: f32,
    attributes: &[(Color, TextWeight)],
) -> Vec<GlyphRun> {
    let metrics = metrics(run, line_height);

    run.glyphs
        .iter()
        .chunk_by(|r| r.metadata)
        .into_iter()
        .map(|(metadata, sub_run_glyphs)| {
            let positioned_glyphs = sub_run_glyphs.map(position_glyph);
            let (color, weight) = attributes[metadata];
            GlyphRun::new(
                translation,
                metrics,
                color,
                weight,
                positioned_glyphs.collect(),
            )
        })
        .collect()
}

pub fn to_glyph_run(translation: Vector3, run: &LayoutRun, line_height: f32) -> GlyphRun {
    let metrics = metrics(run, line_height);
    let positioned = position_glyphs(run.glyphs);
    GlyphRun::new(
        translation,
        metrics,
        Color::BLACK,
        TextWeight::NORMAL,
        positioned,
    )
}

fn metrics(run: &LayoutRun, line_height: f32) -> GlyphRunMetrics {
    let max_ascent = run.line_y - run.line_top;

    GlyphRunMetrics {
        max_ascent: max_ascent.ceil() as _,
        max_descent: (line_height - max_ascent).ceil() as _,
        width: run.line_w.ceil() as u32,
    }
}

/// Position individual `LayoutGlyph` from a `LayoutRun`.
pub fn position_glyphs(glyphs: &[LayoutGlyph]) -> Vec<RunGlyph> {
    glyphs.iter().map(position_glyph).collect()
}

fn position_glyph(glyph: &LayoutGlyph) -> RunGlyph {
    let fractional_pos = if RENDER_SUBPIXEL {
        (glyph.x, glyph.y)
    } else {
        (glyph.x.round(), glyph.y.round())
    };

    // Robustness: There is a function physical() in glyph which also returns a GlyphKey, perhaps
    // use this here.

    let (glyph_key, x, y) = CacheKey::new(
        glyph.font_id,
        glyph.glyph_id,
        glyph.font_size,
        fractional_pos,
        glyph.font_weight,
        CacheKeyFlags::empty(),
    );

    RunGlyph::new((x, y), glyph_key)
}
