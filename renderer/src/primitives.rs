//! Low level render primitives.

use derive_more::Constructor;

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
    Flat,
    Sdf,
}
