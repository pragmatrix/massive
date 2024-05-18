mod bind_group_layout_builder;
mod pipeline;
mod quad_index_buffer;
pub mod texture_sampler;

pub use bind_group_layout_builder::*;
pub use pipeline::*;
pub use quad_index_buffer::*;

use wgpu::BindingResource;

pub trait AsBindingResource {
    fn as_binding_resource(&self) -> wgpu::BindingResource;
}

impl AsBindingResource for wgpu::Sampler {
    fn as_binding_resource(&self) -> wgpu::BindingResource {
        BindingResource::Sampler(self)
    }
}

impl AsBindingResource for wgpu::TextureView {
    fn as_binding_resource(&self) -> wgpu::BindingResource {
        BindingResource::TextureView(self)
    }
}

impl AsBindingResource for wgpu::Buffer {
    fn as_binding_resource(&self) -> wgpu::BindingResource {
        self.as_entire_binding()
    }
}

#[macro_export]
macro_rules! bind_group_entries {
    ($($binding:expr => $resource:expr),*) => {
        &[
            $(
                wgpu::BindGroupEntry {
                    binding: $binding,
                    resource: $crate::tools::AsBindingResource::as_binding_resource($resource),
                },
            )*
        ]
    };
}
