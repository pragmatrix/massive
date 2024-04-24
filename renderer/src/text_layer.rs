use wgpu::BindGroup;

use crate::texture::View;

mod bind_group;

pub use bind_group::*;

/// A layer of 3D text backed by a texture atlas.
pub struct TextLayer {
    // All the fragment shader bindings.
    fragment_shader_bindings: BindGroup,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
}
