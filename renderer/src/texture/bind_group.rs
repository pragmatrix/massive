use derive_more::Deref;

use super::View;
use crate::{ColorBuffer, bind_group_entries, tools::BindGroupLayoutBuilder};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment()
            .texture()
            // Texture size
            .uniform()
            .sampler()
            // Color
            .uniform()
            .build("Texture Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        view: &View,
        texture_sampler: &wgpu::Sampler,
        color: &ColorBuffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Bind Group"),
            layout: &self.0,
            entries: bind_group_entries!(0 => view, 1 => view.size(), 2 => texture_sampler, 3 => color),
        })
    }
}
