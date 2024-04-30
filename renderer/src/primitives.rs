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

    /// How many quads are in this primitive?
    pub fn quads(&self) -> usize {
        match self {
            Self::Texture(_) => 1,
            Self::TextLayer(TextLayer { instance_count, .. }) => *instance_count,
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
