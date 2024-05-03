use super::GlyphClass;
use crate::primitives::Pipeline;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GlyphRasterizationParam {
    pub hinted: bool,
    pub sdf: bool,
}

impl GlyphRasterizationParam {
    pub fn pipeline(&self) -> Pipeline {
        if self.sdf {
            Pipeline::SdfGlyph
        } else {
            Pipeline::PlanarGlyph
        }
    }
}

impl From<GlyphClass> for GlyphRasterizationParam {
    fn from(class: GlyphClass) -> Self {
        use GlyphClass::*;
        match class {
            Zoomed(_) | PixelPerfect { .. } => GlyphRasterizationParam {
                hinted: true,
                sdf: false,
            },
            Distorted(_) => GlyphRasterizationParam {
                hinted: true,
                sdf: true,
            },
        }
    }
}
