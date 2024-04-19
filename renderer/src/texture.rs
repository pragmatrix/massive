use massive_geometry::Point3;
use tracing::{span, Level};
use wgpu::util::DeviceExt;

use crate::{pods::TextureVertex, primitives::Pipeline, texture, ColorBuffer};

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
    #[tracing::instrument(skip_all)]
    pub fn new(
        device: &wgpu::Device,
        pipeline: Pipeline,
        bind_group_layout: &BindGroupLayout,
        texture_sampler: &wgpu::Sampler,
        view: &texture::View,
        color: &ColorBuffer,
        vertices: &[Point3; 4],
    ) -> Self {
        let vertices = points_to_texture_vertices(vertices);

        let vertex_buffer = {
            let span = span!(Level::INFO, "texture-vertex-buffer-creation");
            let _span = span.enter();

            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Texture Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        };

        let bind_group = {
            let span = span!(Level::INFO, "texture-bind-group-creation");
            let _span = span.enter();

            bind_group_layout.create_bind_group(device, view, texture_sampler, color)
        };

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
