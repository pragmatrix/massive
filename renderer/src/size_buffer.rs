use wgpu::util::DeviceExt;

use massive_geometry::SizePx;

use crate::{pods, tools::AsBindingResource};

#[derive(Debug)]
pub struct SizeBuffer(wgpu::Buffer);

impl SizeBuffer {
    pub fn new(device: &wgpu::Device, size: SizePx) -> Self {
        let uniform = pods::TextureSize([size.width as f32, size.height as f32], [0, 0]);

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Size Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self(buffer)
    }
}

impl AsBindingResource for SizeBuffer {
    fn as_binding_resource(&self) -> wgpu::BindingResource<'_> {
        self.0.as_entire_binding()
    }
}
