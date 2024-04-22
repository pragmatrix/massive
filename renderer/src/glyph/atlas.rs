//! A  wgpu glyph atlas for u8 textures. Inspired by glyphon's TextAtlas.
use std::collections::HashMap;

use anyhow::{bail, Result};
use cosmic_text::SwashImage;
use etagere::{Allocation, BucketedAtlasAllocator, Point};
use euclid::size2;

use wgpu::{
    Device, Extent3d, ImageCopyTexture, ImageDataLayout, Origin3d, Queue, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use super::RenderGlyphKey;

struct GlyphAtlas {
    texture: AtlasTexture,
    allocator: BucketedAtlasAllocator,
    /// Storage of the available and (padded) Images.
    images: HashMap<RenderGlyphKey, (Allocation, SwashImage)>,
}

impl GlyphAtlas {
    // TODO: Measure what we usually need and make this a arg to new.
    const INITIAL_SIZE: u32 = 1024;
    const GROWTH_FACTOR: u32 = 2;

    pub fn new(device: &Device) -> Self {
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let dim = Self::INITIAL_SIZE.min(max_texture_dimension_2d);
        let allocator = BucketedAtlasAllocator::new(size2(dim as i32, dim as i32));
        let texture = AtlasTexture::new(device, dim);

        Self {
            texture,
            allocator,
            images: HashMap::default(),
        }
    }

    pub fn exists(&self, key: &RenderGlyphKey) -> bool {
        self.images.contains_key(key)
    }

    /// Makes room and stores a SwashImage in the texture atlas. May reallocate / grow it.
    pub fn store(
        &mut self,
        device: &Device,
        queue: &Queue,
        key: &RenderGlyphKey,
        image: SwashImage,
    ) -> Result<()> {
        debug_assert!(!self.exists(key));

        let (w, h) = (image.placement.width as i32, image.placement.height as i32);
        let size = size2(w, h);

        loop {
            let allocation = self.allocator.allocate(size);
            if let Some(allocation) = allocation {
                debug_assert_eq!(allocation.rectangle.size(), size);
                self.upload(queue, &image, allocation.rectangle.min);
                // commit
                self.images.insert(key.clone(), (allocation, image));
                return Ok(());
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

        // TODO: This allocates the new texture alongside the old for a short period of time.
        // If we won't use COPY_SRC, this should be avoided.
        self.texture = AtlasTexture::new(device, new_dim);
        // After growing, the allocated rectangles retain their position.
        self.allocator.grow(size2(new_dim as i32, new_dim as i32));

        self.upload_all(queue);

        Ok(())
    }

    fn upload_all(&self, queue: &Queue) {
        for (allocation, image) in self.images.values() {
            self.upload(queue, image, allocation.rectangle.min)
        }
    }

    /// Upload the image to the GPU into the atlas texture at the given position.
    fn upload(&self, queue: &Queue, image: &SwashImage, pos: Point) {
        let (x, y) = (pos.x as u32, pos.y as u32);
        let (width, height) = (image.placement.width, image.placement.height);

        queue.write_texture(
            ImageCopyTexture {
                texture: &self.texture.texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            &image.data,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width),
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
    const FORMAT: TextureFormat = TextureFormat::R8Unorm;

    pub fn new(device: &Device, dim: u32) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("Glyph atlas"),
            size: Extent3d {
                width: dim,
                height: dim,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: Self::FORMAT,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&TextureViewDescriptor::default());

        Self { texture, view }
    }

    pub fn dim(&self) -> u32 {
        self.texture.width()
    }
}
