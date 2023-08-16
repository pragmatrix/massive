use super::GlyphClass;
use crate::primitives::Pipeline;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlyphRenderParam {
    // TODO: Add scaling
    pub sdf: bool,
}

impl GlyphRenderParam {
    pub fn pipeline(&self) -> Pipeline {
        if self.sdf {
            Pipeline::Sdf
        } else {
            Pipeline::Flat
        }
    }
}

impl From<GlyphClass> for GlyphRenderParam {
    fn from(class: GlyphClass) -> Self {
        use GlyphClass::*;
        match class {
            Zoomed(_) | PixelPerfect { .. } => GlyphRenderParam { sdf: false },
            Distorted(_) => GlyphRenderParam { sdf: true },
        }
    }
}
