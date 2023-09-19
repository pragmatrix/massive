use granularity_geometry::Point3;
use wgpu::util::DeviceExt;

use crate::{pods::TextureVertex, primitives::Pipeline, texture};

mod bind_group;
mod view;

pub use bind_group::*;
pub use view::*;

/// A texture ready to be rendered.
#[derive(Debug)]
pub struct Texture {
    pub pipeline: Pipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

impl Texture {
    pub fn new(
        device: &wgpu::Device,
        pipeline: Pipeline,
        bind_group_layout: &BindGroupLayout,
        texture_sampler: &wgpu::Sampler,
        view: &texture::View,
        vertices: &[Point3; 4],
    ) -> Self {
        let vertices = points_to_texture_vertices(vertices);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let bind_group = bind_group_layout.create_bind_group(device, view, texture_sampler);

        Self {
            pipeline,
            vertex_buffer,
            bind_group,
        }
    }
}

fn points_to_texture_vertices(points: &[Point3; 4]) -> [TextureVertex; 4] {
    [
        TextureVertex {
            position: points[0].into(),
            tex_coords: [0.0, 0.0],
        },
        TextureVertex {
            position: points[1].into(),
            tex_coords: [0.0, 1.0],
        },
        TextureVertex {
            position: points[2].into(),
            tex_coords: [1.0, 1.0],
        },
        TextureVertex {
            position: points[3].into(),
            tex_coords: [1.0, 0.0],
        },
    ]
}
