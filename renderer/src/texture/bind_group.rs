use crate::tools::BindGroupLayoutBuilder;

use super::View;

use derive_more::Deref;

/// The bind group layout of a texture.
#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment()
            .texture()
            .uniform()
            .sampler()
            .build("Texture Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        view: &View,
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
                    resource: wgpu::BindingResource::Sampler(texture_sampler),
                },
            ],
        })
    }
}
