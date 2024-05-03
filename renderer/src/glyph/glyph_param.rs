use super::GlyphClass;
use crate::primitives::Pipeline;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GlyphRasterizationParam {
    pub sdf: bool,
    pub swash: SwashRasterizationParam,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SwashRasterizationParam {
    pub hinted: bool,
    // Currently used with variable fonts only, by passing the `wght` tag.
    pub weight: swash::Weight,
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
                swash: SwashRasterizationParam {
                    hinted: true,
                    weight: Default::default(),
                },
                sdf: false,
            },
            Distorted(_) => GlyphRasterizationParam {
                swash: SwashRasterizationParam {
                    hinted: true,
                    weight: Default::default(),
                },
                sdf: true,
            },
        }
    }
}
