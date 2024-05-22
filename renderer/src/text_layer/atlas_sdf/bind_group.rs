use derive_more::Deref;
use wgpu::{BindGroup, Device, TextureView};

use crate::{bind_group_entries, tools::BindGroupLayoutBuilder, SizeBuffer};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment()
            .texture()
            // Texture size.
            .uniform()
            .sampler()
            .build("Atlas SDF Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &Device,
        texture_view: &TextureView,
        texture_size: &SizeBuffer,
        texture_sampler: &wgpu::Sampler,
    ) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Atlas SDF Bind Group"),
            layout: &self.0,
            entries: bind_group_entries!(0 => texture_view, 1 => texture_size, 2 => texture_sampler),
        })
    }
}
