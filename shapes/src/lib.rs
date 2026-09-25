mod clip_box;
mod engine;
mod face_metrics;
mod font_manager;
mod font_validation;
mod glyph_run;
mod shape;
mod shaping_engines;
mod text_shaper;

pub use clip_box::*;
pub use engine::{
    FontBytes, FontData, FontRegistry, ShapedCluster, ShapedGlyph, ShapedRun, ShapingEngine,
    ShapingEngineKind, ShapingRequest, TextAttributes, TextFamily, covering_metadata,
    shaped_run_to_glyph_run,
};
pub use face_metrics::FaceMetrics;
pub use font_manager::*;
pub use glyph_run::*;
pub use shape::*;
#[cfg(feature = "cosmic-text")]
pub use shaping_engines::CosmicTextEngine;
#[cfg(feature = "parley")]
pub use shaping_engines::ParleyEngine;
pub use text_shaper::*;

#[cfg(all(not(feature = "parley"), not(feature = "cosmic-text")))]
compile_error!(
    "massive-shapes needs at least one shaping engine feature: `parley` or `cosmic-text`"
);

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

use derive_more::{Deref, DerefMut};

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

    /// Shape with a caller-supplied shaper.
    ///
    /// For code that already holds a shaper.
    pub fn shape_with(self, shaper: &mut Shaper<'_>) -> Option<GlyphRun> {
        self.layouter.layout(shaper, self.font_size)
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
