use super::GlyphClass;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GlyphRasterizationParam {
    // Prefer SDF rasterization if the glyph is monochrome.
    pub prefer_sdf: bool,
    pub swash: SwashRasterizationParam,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SwashRasterizationParam {
    pub hinted: bool,
}

impl From<GlyphClass> for GlyphRasterizationParam {
    fn from(class: GlyphClass) -> Self {
        use GlyphClass::*;
        match class {
            Zoomed(_) | PixelPerfect { .. } => GlyphRasterizationParam {
                swash: SwashRasterizationParam { hinted: true },
                prefer_sdf: false,
            },
            Distorted(_) => GlyphRasterizationParam {
                swash: SwashRasterizationParam { hinted: true },
                prefer_sdf: true,
            },
        }
    }
}
