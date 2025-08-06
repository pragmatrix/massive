use derive_more::Deref;

use crate::{SizeBuffer, bind_group_entries, tools::BindGroupLayoutBuilder};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    #[allow(unused)]
    pub fn new(device: &wgpu::Device) -> Self {
        Self(
            BindGroupLayoutBuilder::fragment_stage()
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
            entries: bind_group_entries!(0 => size),
        })
    }
}
