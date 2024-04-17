use derive_more::Deref;

use super::View;
use crate::{tools::BindGroupLayoutBuilder, ColorBuffer};

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment()
            .texture()
            .uniform()
            .uniform()
            .sampler()
            .build("Texture Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        view: &View,
        color: &ColorBuffer,
        texture_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Bind Group"),
            layout: &self.0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: view.size().as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: color.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(texture_sampler),
                },
            ],
        })
    }
}
