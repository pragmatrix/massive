use derive_more::Deref;
use wgpu::{BindGroup, BindGroupEntry, BindingResource, Device, TextureView};

use crate::{tools::BindGroupLayoutBuilder, SizeBuffer};

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
            .build("Texture Bind Group Layout", device);

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
            label: Some("Text Layer Bind Group"),
            layout: &self.0,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: texture_size.as_binding_resource(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(texture_sampler),
                },
            ],
        })
    }
}
