use std::collections::{
    hash_map::{self, Entry},
    HashMap, HashSet,
};

use anyhow::Result;
use cosmic_text as text;
use log::warn;
use swash::scale::ScaleContext;
use text::SwashContent;

use super::{glyph_param::GlyphRasterizationParam, glyph_rasterization::render_sdf};
use crate::{
    glyph::glyph_rasterization::{pad_image, rasterize_glyph},
    primitives::Pipeline,
    texture,
};

#[derive(Default)]
pub struct GlyphCache {
    scaler: ScaleContext,
    cache: HashMap<RasterizedGlyphKey, Option<RenderGlyph>>,
    retainer: HashSet<RasterizedGlyphKey>,
}

impl GlyphCache {
    /// Returns a `RenderGlyph` and marks this one as used.
    #[tracing::instrument(skip_all)]
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut text::FontSystem,
        key: RasterizedGlyphKey,
    ) -> Option<&RenderGlyph> {
        self.retainer.insert(key.clone());

        match self.cache.entry(key) {
            Entry::Occupied(e) => e.into_mut().as_ref(),
            Entry::Vacant(e) => {
                let glyph = render_glyph(device, queue, font_system, &mut self.scaler, e.key());
                e.insert(glyph).as_ref()
            }
        }
    }

    /// Flushes all the unused glyphs from the cache.
    pub fn flush_unused(&mut self) {
        self.cache.retain(|x, _| self.retainer.contains(x));
        self.retainer.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterizedGlyphKey {
    pub text: text::CacheKey,
    pub param: GlyphRasterizationParam,
}

#[derive(Debug)]
pub struct RenderGlyph {
    pub placement: text::Placement,
    pub pipeline: Pipeline,
    pub texture_view: texture::View,
}

fn render_glyph(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    font_system: &mut text::FontSystem,
    scale_context: &mut ScaleContext,
    key: &RasterizedGlyphKey,
) -> Option<RenderGlyph> {
    // TODO: use rasterize_padded_glyph()!
    let image = rasterize_glyph(font_system, scale_context, key.text, key.param.hinted)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }
    if image.content != SwashContent::Mask {
        warn!("image content type {:?} is unsupported", image.content);
        return None;
    }

    // TODO: propagate errors.
    let (placement, texture_view) =
        image_to_gpu_texture(device, queue, &image, key.param.sdf).ok()?;
    Some(RenderGlyph {
        placement,
        pipeline: key.param.pipeline(),
        texture_view,
    })
}

fn image_to_gpu_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
    sdf: bool,
) -> Result<(text::Placement, texture::View)> {
    if sdf {
        return render_sdf(image)
            .map(|sdf_image| {
                (
                    sdf_image.placement,
                    create_gpu_texture(device, queue, &sdf_image),
                )
            })
            .ok_or_else(|| anyhow::anyhow!("Failed to generate SDF image"));
    }

    // Need to pad the image, otherwise edges may look cut off.
    let padded = pad_image(image);
    Ok((padded.placement, create_gpu_texture(device, queue, &padded)))
}

/// Creates a texture and uploads the image's content to the GPU.
fn create_gpu_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
) -> texture::View {
    let placement = image.placement;
    texture::View::from_data(
        device,
        queue,
        &image.data,
        (placement.width, placement.height),
    )
}
