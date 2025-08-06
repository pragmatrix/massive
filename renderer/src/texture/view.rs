use crate::{SizeBuffer, tools::AsBindingResource};

#[derive(Debug)]
pub struct View {
    view: wgpu::TextureView,
    size: SizeBuffer,
}

impl View {
    /// Creates a texture and uploads the image's content to the GPU.
    pub fn from_data(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        (width, height): (u32, u32),
    ) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: None,
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let size = SizeBuffer::new(device, (width, height));
        Self { view, size }
    }

    pub fn size(&self) -> &SizeBuffer {
        &self.size
    }
}

impl AsBindingResource for View {
    fn as_binding_resource(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::TextureView(&self.view)
    }
}
