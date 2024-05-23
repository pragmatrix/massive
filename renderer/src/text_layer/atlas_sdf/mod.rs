//! An atlas based SDF renderer.

mod bind_group;
mod renderer;

pub use bind_group::*;
use massive_geometry::{Color, Matrix4, Point3};
pub use renderer::*;

use crate::glyph::glyph_atlas;

pub struct QuadBatch {
    // Matrix is not prepared as a buffer, because it is combined with the camera matrix before
    // uploading to the shader.
    model_matrix: Matrix4,
    fs_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    quad_count: usize,
}

#[derive(Debug)]
pub struct QuadInstance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Point3; 4],
    pub color: Color,
}
