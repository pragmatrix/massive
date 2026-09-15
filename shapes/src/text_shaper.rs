//! Ergonomic shaping: builder API over the engine-neutral request model.

use std::ops::Range;

use massive_geometry::Color;

use crate::engine::{ShapingRequest, TextAttributes, TextFamily};
use crate::{GlyphRun, Shaper, TextWeight};

/// A text shaper builder: attributed text that resolves to a [`GlyphRun`].
#[derive(Debug)]
pub struct TextShaper<'a> {
    text: &'a str,
    default_attributes: TextAttributes<'a>,
    range_attributes: Vec<(Range<usize>, TextAttributes<'a>)>,
}

impl<'a> TextShaper<'a> {
    /// Creates a default text shaper that uses the Sans-Serif family.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            default_attributes: TextAttributes::default(),
            range_attributes: Vec::new(),
        }
    }

    pub fn with_default_attributes(mut self, attributes: TextAttributes<'a>) -> Self {
        self.default_attributes = attributes;
        self
    }

    pub fn with_family(mut self, family: impl Into<TextFamily<'a>>) -> Self {
        self.default_attributes.family = family.into();
        self
    }

    pub fn with_weight(mut self, weight: TextWeight) -> Self {
        self.default_attributes.weight = weight;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.default_attributes.color = color;
        self
    }

    pub fn add_range_attributes(&mut self, range: Range<usize>, attributes: TextAttributes<'a>) {
        self.range_attributes.push((range, attributes))
    }

    /// Shape the first line of the text at `font_size` pixels.
    pub fn layout(self, shaper: &mut Shaper<'_>, font_size: f32) -> Option<GlyphRun> {
        let mut request = ShapingRequest::new(self.text, self.default_attributes.clone());
        request.ranges = self.range_attributes;
        shaper.glyph_run(&request, font_size)
    }
}
