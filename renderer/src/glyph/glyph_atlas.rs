//! A  wgpu glyph atlas for u8 textures. Inspired by glyphon's TextAtlas.
use std::collections::HashMap;

use anyhow::{bail, Result};
use cosmic_text::{SwashContent, SwashImage};
pub use etagere::Rectangle;
use etagere::{Allocation, BucketedAtlasAllocator, Point};
use euclid::size2;

use tracing::instrument;
use wgpu::{
    Device, Extent3d, Origin3d, Queue, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};

use super::RasterizedGlyphKey;

pub struct GlyphAtlas {
    texture: AtlasTexture,
    allocator: BucketedAtlasAllocator,
    /// Storage of the available and (padded) Images.
    images: HashMap<RasterizedGlyphKey, (Allocation, SwashImage)>,
}

impl GlyphAtlas {
    // TODO: Measure what we usually need and make this a arg to new.
    const INITIAL_SIZE: u32 = 128;
    const GROWTH_FACTOR: u32 = 2;

    pub fn new(device: &Device, texture_format: TextureFormat) -> Self {
        assert!(
            texture_format == TextureFormat::R8Unorm || texture_format == TextureFormat::Rgba8Unorm
        );

        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let dim = Self::INITIAL_SIZE.min(max_texture_dimension_2d);
        let allocator = BucketedAtlasAllocator::new(size2(dim as i32, dim as i32));
        let texture = AtlasTexture::new(device, texture_format, dim);

        Self {
            texture,
            allocator,
            images: HashMap::default(),
        }
    }

    #[allow(unused)]
    pub fn size(&self) -> (u32, u32) {
        let dim = self.texture.dim();
        (dim, dim)
    }

    pub fn texture_view(&self) -> &TextureView {
        self.texture.view()
    }

    pub fn get(&self, key: &RasterizedGlyphKey) -> Option<(Rectangle, &SwashImage)> {
        self.images.get(key).map(|(a, image)| {
            let image_size = size2(image.placement.width as i32, image.placement.height as i32);
            (
                Rectangle::new(a.rectangle.min, a.rectangle.min + image_size),
                image,
            )
        })
    }

    /// Makes room and stores a SwashImage in the texture atlas. May reallocate / grow it.
    pub fn store(
        &mut self,
        device: &Device,
        queue: &Queue,
        key: &RasterizedGlyphKey,
        image: SwashImage,
    ) -> Result<Rectangle> {
        debug_assert!(!self.images.contains_key(key));

        let size = size2(image.placement.width as i32, image.placement.height as i32);

        loop {
            let allocation = self.allocator.allocate(size);
            if let Some(allocation) = allocation {
                // Allocation might be larger, so we can't return the rectangles directly.
                let allocated_size = allocation.rectangle.size();
                debug_assert!(allocated_size.width >= size.width);
                debug_assert!(allocated_size.height >= size.height);
                self.upload(queue, &image, allocation.rectangle.min);
                // commit
                self.images.insert(key.clone(), (allocation, image));
                let final_rect =
                    Rectangle::new(allocation.rectangle.min, allocation.rectangle.min + size);
                return Ok(final_rect);
            }

            self.grow(device, queue)?
        }
    }

    fn grow(&mut self, device: &Device, queue: &Queue) -> Result<()> {
        // TODO: allocate additional textures, if this fails. TODO: try to copy from texture to
        // texture when growing (COPY_SRC). Does this cost performance, measure on all backends?

        let current_dim = self.texture.dim();

        let new_dim =
            (current_dim * Self::GROWTH_FACTOR).min(device.limits().max_texture_dimension_2d);

        if new_dim == current_dim {
            // TODO: Support multiple atlas textures.
            bail!("Atlas reached its maximum size of {current_dim}x{current_dim}");
        }

        log::info!("Growing glyph atlas from {current_dim} to {new_dim}");

        // TODO: This allocates the new texture alongside the old for a short period of time.
        // If we won't use COPY_SRC, this should be avoided.
        self.texture = AtlasTexture::new(device, self.texture.format(), new_dim);
        // After growing, the allocated rectangles retain their position.
        self.allocator.grow(size2(new_dim as i32, new_dim as i32));

        self.upload_all(queue);

        Ok(())
    }

    #[instrument(skip_all)]
    fn upload_all(&self, queue: &Queue) {
        for (allocation, image) in self.images.values() {
            self.upload(queue, image, allocation.rectangle.min)
        }
    }

    /// Upload the image to the GPU into the atlas texture at the given position.
    #[instrument(skip_all)]
    fn upload(&self, queue: &Queue, image: &SwashImage, pos: Point) {
        let (x, y) = (pos.x as u32, pos.y as u32);
        let (width, height) = (image.placement.width, image.placement.height);

        let bytes_per_pixel = match image.content {
            SwashContent::Mask => 1,
            SwashContent::SubpixelMask => panic!("Unsupported Subpixel Mask Image"),
            SwashContent::Color => 4,
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture.texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            &image.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_pixel),
                rows_per_image: None,
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[derive(Debug)]
struct AtlasTexture {
    texture: Texture,
    view: TextureView,
}

impl AtlasTexture {
    pub fn new(device: &Device, texture_format: TextureFormat, dim: u32) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: Extent3d {
                width: dim,
                height: dim,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: texture_format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&TextureViewDescriptor::default());

        Self { texture, view }
    }

    pub fn format(&self) -> TextureFormat {
        self.texture.format()
    }

    pub fn dim(&self) -> u32 {
        self.texture.width()
    }

    pub fn view(&self) -> &TextureView {
        &self.view
    }
}
