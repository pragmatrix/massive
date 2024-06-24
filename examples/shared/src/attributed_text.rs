use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};
use massive_geometry::{Color, Vector3};
use massive_shapes::{GlyphRun, TextWeight};
use serde::{Deserialize, Serialize};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::positioning;

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

pub fn shape_text(
    font_system: &mut FontSystem,
    text: &str,
    attributes: &[TextAttribute],
    font_size: f32,
    line_height: f32,
) -> (Vec<GlyphRun>, f64) {
    syntax::assert_covers_all_text(
        &attributes
            .iter()
            .map(|ta| ta.range.clone())
            .collect::<Vec<_>>(),
        text.len(),
    );

    // The text covers everything. But if these attributes are appearing without adjusted metadata,
    // something is wrong. Set it to an illegal offset `usize::MAX` for now.
    let base_attrs = Attrs::new().family(Family::Monospace).metadata(usize::MAX);
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(font_system, f32::INFINITY, f32::INFINITY);
    // buffer.set_text(font_system, text, attrs, Shaping::Advanced);
    buffer.set_wrap(font_system, Wrap::None);
    // Create associated metadata.
    let text_attr_spans = attributes.iter().enumerate().map(|(attribute_index, ta)| {
        (
            text.get(ta.range.clone()).unwrap(),
            base_attrs.metadata(attribute_index),
        )
    });
    buffer.set_rich_text(font_system, text_attr_spans, base_attrs, Shaping::Advanced);
    buffer.shape_until_scroll(font_system, true);

    let mut runs = Vec::new();
    let mut height: f64 = 0.;

    let attributes: Vec<_> = attributes.iter().map(|ta| (ta.color, ta.weight)).collect();

    for run in buffer.layout_runs() {
        // Lines are positioned on line_height.
        let translation = Vector3::new(0., run.line_top as f64, 0.);
        for run in
            positioning::to_attributed_glyph_runs(translation, &run, line_height, &attributes)
        {
            runs.push(run);
        }
        height = height.max(translation.y + line_height as f64);
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
}
