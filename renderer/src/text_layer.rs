use massive_geometry::Matrix4;
use wgpu::BindGroup;

mod bind_group;

pub use bind_group::*;

/// A layer of 3D text backed by a texture atlas.
pub struct TextLayer {
    // Matrix is not supplied as a buffer, because it is combined with the camera matrix before
    // uploading to the shader.
    pub model_matrix: Matrix4,
    pub fragment_shader_bind_group: BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub quad_count: usize,
}
