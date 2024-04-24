use derive_more::Deref;
use wgpu::{BindGroup, BindGroupEntry, BindingResource, Device};

use crate::{texture::View, tools::BindGroupLayoutBuilder};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment()
            .texture()
            // Texture size
            .uniform()
            .sampler()
            .build("Texture Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &Device,
        view: &View,
        texture_sampler: &wgpu::Sampler,
    ) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Text Layer Bind Group"),
            layout: &self.0,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: view.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: view.size().as_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(texture_sampler),
                },
            ],
        })
    }
}
