use granularity_geometry::Point3;
use wgpu::util::DeviceExt;

use crate::{
    command::{ImageData, Pipeline},
    pods::TextureVertex,
};

mod bind_group_layout;
mod size;
mod view;

pub use bind_group_layout::*;
pub use size::*;
pub use view::*;

/// A texture ready to be rendered.
#[derive(Debug)]
pub struct Texture {
    pipeline: Pipeline,
    vertex_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Texture {
    pub fn from_vertices_and_image_data(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: Pipeline,
        bind_group_layout: &BindGroupLayout,
        vertices: &[Point3; 4],
        image_data: &ImageData,
        texture_sampler: &wgpu::Sampler,
    ) -> Self {
        let view = View::from_image_data(device, queue, image_data);
        let vertices = create_texture_vertices(vertices);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let bind_group = bind_group_layout.create_bind_group(device, &view, texture_sampler);

        Self {
            pipeline,
            vertex_buffer,
            bind_group,
        }
    }
}

fn create_texture_vertices(points: &[Point3; 4]) -> [TextureVertex; 4] {
    [
        TextureVertex {
            position: points[0].into(),
            tex_coords: [0.0, 0.0],
        },
        TextureVertex {
            position: points[1].into(),
            tex_coords: [1.0, 0.0],
        },
        TextureVertex {
            position: points[2].into(),
            tex_coords: [1.0, 1.0],
        },
        TextureVertex {
            position: points[3].into(),
            tex_coords: [0.0, 1.0],
        },
    ]
}

fn create_texture_size_buffer(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Texture Size Buffer"),
        contents: bytemuck::cast_slice(&[size.0, size.1]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}
