use wgpu::util::DeviceExt;

use crate::pods;

#[derive(Debug)]
pub struct SizeBuffer(wgpu::Buffer);

impl SizeBuffer {
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let uniform = pods::TextureSize([size.0 as f32, size.1 as f32], [0, 0]);

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Size Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self(buffer)
    }

    pub fn as_binding_resource(&self) -> wgpu::BindingResource {
        self.0.as_entire_binding()
    }
}
