//! Low level render primitives.

use crate::texture::Texture;

pub enum Primitive {
    Texture(Texture),
}

impl Primitive {
    pub fn pipeline(&self) -> Pipeline {
        match self {
            Self::Texture(Texture { pipeline, .. }) => *pipeline,
        }
    }

    /// How many quads are in this primitive?
    pub fn quads(&self) -> usize {
        match self {
            Self::Texture(_) => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pipeline {
    PlanarGlyph,
    SdfGlyph,
    TextLayer,
    Circle,
    RoundedRect,
}
