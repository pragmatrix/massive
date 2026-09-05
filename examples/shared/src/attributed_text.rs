use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use massive_geometry::{Color, Vector3};
use massive_shapes::{GlyphBrush, GlyphRun, TextWeight, glyph_run_to_run, line_runs};
use massive_shell::FontManager;
use parley::{Alignment, FontWeight, LineHeight, StyleProperty};

/// A serializable representation of highlighted code.
#[derive(Debug, Serialize, Deserialize)]
pub struct AttributedText {
    pub text: String,
    pub attributes: Vec<TextAttribute>,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct TextAttribute {
    pub range: Range<usize>,
    pub color: Color,
    pub weight: TextWeight,
}

/// Shape `text` into [`GlyphRun`]s, honoring the given per-attribute weights/colors.
///
/// Layout is driven by Parley through the manager's shared font + layout contexts. Each line is
/// translated down by `line_height`, matching the legacy cosmic-text behavior, and each returned
/// run carries the color of the attribute covering its text span.
pub fn shape_text(
    fonts: &FontManager,
    text: &str,
    attributes: &[TextAttribute],
    font_size: f32,
    line_height: f32,
    translation: impl Into<Option<Vector3>>,
) -> (Vec<GlyphRun>, f64) {
    syntax::assert_covers_all_text(
        &attributes
            .iter()
            .map(|ta| ta.range.clone())
            .collect::<Vec<_>>(),
        text.len(),
    );

    let translation = translation.into().unwrap_or(Vector3::new(0., 0., 0.));

    fonts.with_shape(|fcx, lcx, resolver| {
        let mut builder = lcx.ranged_builder(fcx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontFamily(parley::FontFamily::named("monospace")));
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height)));
        for ta in attributes {
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(ta.weight.0 as f32)),
                ta.range.clone(),
            );
        }

        let mut layout: parley::Layout<GlyphBrush> = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, Default::default());

        let mut runs = Vec::new();
        let mut height: f64 = 0.;

        // Lines are positioned on `line_height` (matching the legacy `run.line_top`).
        for (index, line) in layout.lines().enumerate() {
            let line_top = index as f64 * line_height as f64;
            let line_translation = translation + Vector3::new(0., line_top, 0.);
            for parley_run in line_runs(&line) {
                let color = color_for_range(parley_run.run().text_range(), attributes);
                let run = glyph_run_to_run(
                    parley_run,
                    resolver,
                    Color::BLACK,
                    TextWeight::NORMAL,
                    line_translation,
                )
                .with_color(color);
                runs.push(run);
            }
            height = height.max(line_top + line_height as f64);
        }

        (runs, height)
    })
}

/// Look up the color of the attribute covering `range`.
fn color_for_range(range: Range<usize>, attributes: &[TextAttribute]) -> Color {
    attributes
        .iter()
        .find(|ta| ta.range.start <= range.start && range.end <= ta.range.end)
        .map(|ta| ta.color)
        .unwrap_or(Color::BLACK)
}

mod syntax {
    use std::ops::Range;

    pub fn assert_covers_all_text(ranges: &[Range<usize>], text_len: usize) {
        if text_len == 0 {
            return;
        }
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[ranges.len() - 1].end, text_len);
        assert_contiguous(ranges);
    }

    pub fn assert_contiguous(range: &[Range<usize>]) {
        for i in range.windows(2) {
            assert!(i[0].end == i[1].start)
        }
    }
}
