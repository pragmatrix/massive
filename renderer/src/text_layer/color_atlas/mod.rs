//! An atlas based SDF renderer.

mod bind_group;
mod renderer;

pub use bind_group::*;
use massive_geometry::Point3;
pub use renderer::*;

use crate::glyph::glyph_atlas;

#[derive(Debug)]
pub struct QuadInstance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Point3; 4],
}
