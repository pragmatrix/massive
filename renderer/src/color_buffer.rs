use massive_geometry::Color;
use wgpu::util::DeviceExt;

use crate::pods;

#[derive(Debug)]
pub struct ColorBuffer(wgpu::Buffer);

impl ColorBuffer {
    pub fn new(device: &wgpu::Device, color: Color) -> Self {
        let uniform = pods::Color([color.red, color.green, color.blue, color.alpha]);

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Color Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self(buffer)
    }

    pub fn as_binding(&self) -> wgpu::BindingResource {
        self.0.as_entire_binding()
    }
}
