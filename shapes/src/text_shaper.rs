use std::ops::Range;

use parley::StyleProperty;

use massive_geometry::Color;

use crate::{GlyphBrush, GlyphRun, Shaper, TextWeight, line_to_run};

#[derive(Debug)]
pub struct TextShaper<'a> {
    text: &'a str,
    /// Architecture: Could we use lifetimes here too (e.g. FontFamily string refs).
    default_attributes: TextAttributes<'a>,
    range_attributes: Vec<(Range<usize>, TextAttributes<'a>)>,
}

#[derive(Debug)]
pub struct TextAttributes<'a> {
    family: parley::FontFamily<'a>,
    weight: TextWeight,
    color: Color,
}

impl Default for TextAttributes<'_> {
    fn default() -> Self {
        Self {
            family: parley::FontFamily::Source(std::borrow::Cow::Borrowed("sans-serif")),
            weight: TextWeight::default(),
            color: Color::BLACK,
        }
    }
}

impl<'a> TextAttributes<'a> {
    pub fn with_family(mut self, family: parley::FontFamily<'a>) -> Self {
        self.family = family;
        self
    }

    pub fn with_weight(mut self, weight: TextWeight) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
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

    pub fn add_range_attributes(&mut self, range: Range<usize>, attributes: TextAttributes<'a>) {
        self.range_attributes.push((range, attributes))
    }

    // Feature: Why is there only one FontSize here? Parley now supports per-span font sizes via
    // `StyleProperty::FontSize`, but `TextShaper` doesn't expose them yet (uniform `font_size` only
    // for now; revisit to satisfy this TODO).
    pub fn layout(self, shaper: &mut Shaper<'_>, font_size: f32) -> Option<GlyphRun> {
        let (font_context, layout_context) = shaper.contexts();
        let mut builder = layout_context.ranged_builder(font_context, self.text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontFamily(
            self.default_attributes.family.clone(),
        ));
        builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(
            self.default_attributes.weight.0 as f32,
        )));
        for (range, attrs) in &self.range_attributes {
            builder.push(
                StyleProperty::FontFamily(attrs.family.clone()),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontWeight(parley::FontWeight::new(attrs.weight.0 as f32)),
                range.clone(),
            );
        }
        let mut layout: parley::Layout<GlyphBrush> = builder.build(self.text);
        layout.break_all_lines(None);
        layout.align(parley::Alignment::Start, Default::default());

        // Feature: Support multi-line layout.
        let line = layout.get(0)?;

        Some(line_to_run(
            &line,
            self.default_attributes.color,
            self.default_attributes.weight,
            Default::default(),
        ))
    }
}
