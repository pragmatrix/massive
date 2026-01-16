use std::ops::Range;

use cosmic_text::{
    Attrs, AttrsList, BufferLine, Family, FontSystem, LayoutGlyph, LayoutLine, LineEnding, Shaping,
    Weight, Wrap,
};

use massive_geometry::Color;

use crate::{GlyphKey, GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

#[derive(Debug)]
pub struct TextLayouter<'a> {
    text: &'a str,
    /// Architecture: Could we use lifetimes here too (e.g. FontFamily string refs).
    default_attributes: TextAttributes<'a>,
    range_attributes: Vec<(Range<usize>, TextAttributes<'a>)>,
}

#[derive(Debug)]
pub struct TextAttributes<'a> {
    family: Family<'a>,
    weight: TextWeight,
    color: Color,
}

impl Default for TextAttributes<'_> {
    fn default() -> Self {
        Self {
            family: Family::SansSerif,
            weight: TextWeight::default(),
            color: Color::BLACK,
        }
    }
}

impl<'a> TextAttributes<'a> {
    pub fn with_family(mut self, family: Family<'a>) -> Self {
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

    fn to_attrs(&self) -> Attrs<'a> {
        // Performance: Don't add defaults.
        Attrs::new()
            .family(self.family)
            .weight(Weight(self.weight.0))
    }
}

impl<'a> TextLayouter<'a> {
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

    // Feature: Why is there only one FontSize here? Check out parley.
    pub fn layout(self, font_system: &mut FontSystem, font_size: f32) -> Option<GlyphRun> {
        // Performance: BufferLine makes a copy of the text, is there a better way?
        // Performance: Under the hood, HarfRust is used for text shaping, use it directly?
        // Performance: Shaping maintains internal caches, which might benefit reusing them.
        let mut attrs_list = AttrsList::new(&self.default_attributes.to_attrs());
        for (range, attrs) in self.range_attributes {
            attrs_list.add_span(range, &attrs.to_attrs());
        }

        let mut buffer =
            BufferLine::new(self.text, LineEnding::None, attrs_list, Shaping::Advanced);

        // let shaped_glyphs = buffer
        //     // Simplify: If the ShapeLine cache is always empty, we may be able to use
        //     // ShapeLine::build directly, or even better cache it directly here? This will then
        //     // reuse most allocations? ... but we could just re-use BufferLine, or....?
        //     .shape(font_system, 0 /* tab size */)
        //     .spans
        //     .iter()
        //     .flat_map(|span| &span.words)
        //     .filter(|word| !word.blank)
        //     .flat_map(|word| &word.glyphs);

        let layouted_lines = buffer.layout(font_system, font_size, None, Wrap::None, None, 0);
        if layouted_lines.is_empty() {
            return None;
        }

        // Use the first line for metrics (for now).
        // Feature: Support multi-line layout.
        let metrics_line = &layouted_lines[0];
        let metrics = metrics(metrics_line);

        let layouted_glyphs = layouted_lines.iter().flat_map(|l| &l.glyphs);

        // Performance: Is there a better way to estimate the number of resulting glyphs?
        let mut glyphs = Vec::with_capacity(self.text.len());
        for glyph in layouted_glyphs {
            // Optimization: Don't pass empty / blank glyphs.
            glyphs.push(position_glyph(glyph));
        }

        Some(GlyphRun {
            translation: Default::default(),
            metrics,
            text_color: self.default_attributes.color,
            // This looks redundant here. Isn't this specified by each Glyph?
            text_weight: self.default_attributes.weight,
            glyphs,
        })
    }
}

fn metrics(line: &LayoutLine) -> GlyphRunMetrics {
    // This assumes that we can derive the line height from the LayoutLine directly.
    // Robustness: May need some more parameterization here.
    let max_ascent = line.max_ascent;
    let max_decent = line.max_descent;

    GlyphRunMetrics {
        max_ascent: max_ascent.ceil() as _,
        max_descent: max_decent.ceil() as _,
        width: line.w.ceil() as u32,
    }
}

fn position_glyph(glyph: &LayoutGlyph) -> RunGlyph {
    let pos = (glyph.x.round() as i32, glyph.y.round() as i32);

    // Robustness: There is a function physical() in glyph which also returns a GlyphKey, perhaps
    // use this here.

    RunGlyph::new(
        pos,
        GlyphKey::new(
            glyph.font_id,
            glyph.glyph_id,
            glyph.font_size,
            TextWeight(glyph.font_weight.0),
        ),
    )
}
