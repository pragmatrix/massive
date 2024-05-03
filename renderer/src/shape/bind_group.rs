use derive_more::Deref;

use crate::{tools::BindGroupLayoutBuilder, SizeBuffer};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        Self(
            BindGroupLayoutBuilder::fragment()
                // Size of shape.
                .uniform()
                .build("Shape Bind Group Layout", device),
        )
    }

    #[allow(dead_code)]
    pub fn create_bind_group(&self, device: &wgpu::Device, size: &SizeBuffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shape Bind Group"),
            layout: &self.0,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: size.as_binding_resource(),
            }],
        })
    }
}
