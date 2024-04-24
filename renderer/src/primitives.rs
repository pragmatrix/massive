//! Low level render primitives.

use crate::{text_layer::TextLayer, texture::Texture};

pub enum Primitive {
    Texture(Texture),
    TextLayer(TextLayer),
}

impl Primitive {
    pub fn pipeline(&self) -> Pipeline {
        match self {
            Self::Texture(Texture { pipeline, .. }) => *pipeline,
            Self::TextLayer { .. } => Pipeline::TextLayer,
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
