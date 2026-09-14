//! Multi-line attributed text shaping on the engine-neutral shaping contract.
//!
//! Each line is shaped separately and positioned on `line_height` steps, matching the legacy
//! cosmic-text behavior; each returned [`GlyphRun`] carries the color of the attribute covering
//! its text span.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use massive_geometry::{Color, Vector3};

use massive_shapes::{
    GlyphRun, Shaper, ShapingRequest, TextAttributes, TextFamily, TextWeight,
    shaped_run_to_glyph_run,
};

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

/// Shape `text` into [`GlyphRun`]s, one per line, honoring per-attribute weights/colors.
///
/// The text is split into lines (byte offsets stay aligned with the original text), each line's
/// attribute ranges are re-based locally, and each shaped run is translated down by `line_height`
/// per line index. The returned height covers all lines.
pub fn shape_text(
    shaper: &mut Shaper<'_>,
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

    let mut runs = Vec::new();
    let mut height: f64 = 0.;

    for (index, (line_offset, line_text)) in syntax::split_lines(text).enumerate() {
        let default_attributes = TextAttributes::default()
            .with_family(TextFamily::Monospace)
            .with_weight(TextWeight::NORMAL);
        let mut request = ShapingRequest::new(line_text, default_attributes);
        // Re-map the attribute ranges covering this line onto the line's local byte range.
        for ta in attributes {
            let start = ta.range.start.saturating_sub(line_offset);
            let end = ta
                .range
                .end
                .saturating_sub(line_offset)
                .min(line_text.len());
            if start >= end {
                continue;
            }
            request.ranges.push((
                start..end,
                TextAttributes::default()
                    .with_weight(ta.weight)
                    .with_color(ta.color),
            ));
        }

        let line_top = index as f64 * line_height as f64;
        let line_translation = translation + Vector3::new(0., line_top, 0.);
        if let Some(run) = shaper.shape(&request, font_size) {
            runs.push(shaped_run_to_glyph_run(
                &run,
                Color::BLACK,
                TextWeight::NORMAL,
                line_translation,
            ));
        }
        height = height.max(line_top + line_height as f64);
    }

    (runs, height)
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

    /// Split `text` into lines as `(byte_offset, line)` keeping the trailing `\n` on its line so
    /// per-attribute byte ranges re-base without remapping.
    pub fn split_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
        let mut rest = Some(text);
        let mut offset = 0;
        std::iter::from_fn(move || {
            let current = rest?;
            if current.is_empty() {
                rest = None;
                return None;
            }
            match current.find('\n') {
                Some(pos) => {
                    rest = Some(&current[pos + 1..]);
                    let line = &current[..=pos];
                    let start = offset;
                    offset += line.len();
                    Some((start, line))
                }
                None => {
                    rest = None;
                    let start = offset;
                    offset += current.len();
                    Some((start, current))
                }
            }
        })
    }
}
