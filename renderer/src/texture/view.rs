use crate::command::ImageData;

use super::Size;

#[derive(Debug)]
pub struct View {
    view: wgpu::TextureView,
    size: Size,
}

impl View {
    /// Creates a texture and uploads the image's content to the GPU.
    pub fn from_image_data(device: &wgpu::Device, queue: &wgpu::Queue, image: &ImageData) -> Self {
        let size = wgpu::Extent3d {
            width: image.size.0,
            height: image.size.1,
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
            &image.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(image.size.0),
                rows_per_image: None,
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let size = Size::new(device, image.size);
        Self { view, size }
    }

    pub fn size(&self) -> &Size {
        &self.size
    }

    pub fn as_binding(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::TextureView(&self.view)
    }
}
