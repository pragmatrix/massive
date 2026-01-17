mod glyph_run;
mod shape;
mod text_shaper;

use derive_more::{Deref, DerefMut};
pub use glyph_run::*;
pub use shape::*;
pub use text_shaper::*;

use cosmic_text::FontSystem;

// Ergonomics

pub trait Layout<'b> {
    fn layout<'a>(self) -> TextShaper<'a>
    where
        'b: 'a;
}

impl<'b> Layout<'b> for &'b String {
    fn layout<'a>(self) -> TextShaper<'a>
    where
        'b: 'a,
    {
        self.as_str().layout()
    }
}

impl<'b> Layout<'b> for &'b str {
    fn layout<'a>(self) -> TextShaper<'a>
    where
        'b: 'a,
    {
        TextShaper::new(self)
    }
}

// Robustness: I am not so sure about the DerefMut, because some functions take self in TextLayouter.
#[derive(Debug, Deref, DerefMut)]
pub struct SizedTextShaper<'a> {
    #[deref]
    #[deref_mut]
    layouter: TextShaper<'a>,
    font_size: f32,
}

impl<'a> SizedTextShaper<'a> {
    pub fn new(text: &'a str, font_size: f32) -> Self {
        Self {
            layouter: text.layout(),
            font_size,
        }
    }

    pub fn shape(self, font_system: &mut FontSystem) -> Option<GlyphRun> {
        self.layouter.layout(font_system, self.font_size)
    }
}

pub trait Size<'b> {
    fn size<'a>(self, font_size: f32) -> SizedTextShaper<'a>
    where
        'b: 'a;
}

impl<'b> Size<'b> for &'b String {
    fn size<'a>(self, font_size: f32) -> SizedTextShaper<'a>
    where
        'b: 'a,
    {
        self.as_str().size(font_size)
    }
}

impl<'b> Size<'b> for &'b str {
    fn size<'a>(self, font_size: f32) -> SizedTextShaper<'a>
    where
        'b: 'a,
    {
        SizedTextShaper::new(self, font_size)
    }
}
