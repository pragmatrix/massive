use super::Size;

#[derive(Debug)]
pub struct View {
    view: wgpu::TextureView,
    size: Size,
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
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("Texture"),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: None,
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let size = Size::new(device, (width, height));
        Self { view, size }
    }

    pub fn size(&self) -> &Size {
        &self.size
    }

    pub fn as_binding(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::TextureView(&self.view)
    }
}
