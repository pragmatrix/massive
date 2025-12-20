//! A  wgpu glyph atlas for u8 textures. Inspired by glyphon's TextAtlas.
use std::{collections::HashMap, fmt};

use anyhow::{Result, bail};
use cosmic_text::{Placement, SwashContent, SwashImage};
pub use etagere::Rectangle;
use etagere::{Allocation, BucketedAtlasAllocator, Point};
use euclid::size2;

use massive_geometry::SizePx;
use tracing::instrument;
use wgpu::{
    Device, Extent3d, Origin3d, Queue, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use crate::glyph::glyph_rasterization::RasterizedGlyphKey;

pub struct GlyphAtlas {
    texture: AtlasTexture,
    allocator: BucketedAtlasAllocator,
    /// Storage of the available and (padded) Images.
    images: HashMap<RasterizedGlyphKey, (Allocation, Placement)>,
}

impl fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("texture", &self.texture)
            .field("images", &self.images)
            .finish()
    }
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
    pub fn size(&self) -> SizePx {
        let dim = self.texture.dim();
        (dim, dim).into()
    }

    pub fn texture_view(&self) -> &TextureView {
        self.texture.view()
    }

    // Optimization: Size of allocation rectangle is _always_ equal to the size of the placement.
    pub fn get(&self, key: &RasterizedGlyphKey) -> Option<(Rectangle, Placement)> {
        self.images.get(key).map(|(allocation, placement)| {
            let image_size = size2(placement.width as i32, placement.height as i32);
            (
                Rectangle::new(
                    allocation.rectangle.min,
                    allocation.rectangle.min + image_size,
                ),
                *placement,
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
                self.copy_image_to_atlas(queue, &image, allocation.rectangle.min);
                // commit
                self.images
                    .insert(key.clone(), (allocation, image.placement));
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

        let new_texture = AtlasTexture::new(device, self.texture.format(), new_dim);
        Self::copy_texture(
            device,
            queue,
            self.texture.texture(),
            new_texture.texture(),
            (current_dim, current_dim).into(),
        );
        self.texture = new_texture;
        // After growing, the allocated rectangles retain their position.
        self.allocator.grow(size2(new_dim as i32, new_dim as i32));

        // Performance: Just copy from the old texture and then throw it away?
        // self.upload_all(queue);

        Ok(())
    }

    fn copy_texture(device: &Device, queue: &Queue, from: &Texture, to: &Texture, size: SizePx) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Atlas copy encoder"),
        });
        encoder.copy_texture_to_texture(
            TexelCopyTextureInfo {
                texture: from,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyTextureInfo {
                texture: to,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
    }

    /// Upload the image to the GPU into the atlas texture at the given position.
    #[instrument(skip_all)]
    fn copy_image_to_atlas(&self, queue: &Queue, image: &SwashImage, pos: Point) {
        let (x, y) = (pos.x as u32, pos.y as u32);
        let (width, height) = (image.placement.width, image.placement.height);

        let bytes_per_pixel = match image.content {
            SwashContent::Mask => 1,
            SwashContent::SubpixelMask => panic!("Unsupported Subpixel Mask Image"),
            SwashContent::Color => 4,
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.texture.texture(),
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
            usage: TextureUsages::TEXTURE_BINDING
                // COPY_SRC is needed when this texture gets to small and needs to grow.
                | TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&TextureViewDescriptor::default());

        Self { view }
    }

    pub fn format(&self) -> TextureFormat {
        self.texture().format()
    }

    pub fn dim(&self) -> u32 {
        self.texture().width()
    }

    pub fn texture(&self) -> &Texture {
        self.view.texture()
    }

    pub fn view(&self) -> &TextureView {
        &self.view
    }
}
