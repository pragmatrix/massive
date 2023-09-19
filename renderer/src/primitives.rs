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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pipeline {
    PlanarGlyph,
    SdfGlyph,
    Circle,
    RoundedRect,
}
