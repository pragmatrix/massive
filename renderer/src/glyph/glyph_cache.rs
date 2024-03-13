use std::collections::{hash_map, HashMap, HashSet};

use anyhow::Result;
use cosmic_text as text;
use swash::scale::ScaleContext;

use super::{glyph_image_renderer::render_sdf, glyph_param::GlyphRenderParam};
use crate::{
    glyph::glyph_image_renderer::{pad_image, render_glyph_image},
    primitives::Pipeline,
    texture,
};

#[derive(Default)]
pub struct GlyphCache {
    scaler: ScaleContext,
    cache: HashMap<RenderGlyphKey, Option<RenderGlyph>>,
    retainer: HashSet<RenderGlyphKey>,
}

impl GlyphCache {
    /// Returns a `RenderGlyph` and marks this one as used.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut text::FontSystem,
        glyph_key: text::CacheKey,
        glyph_param: GlyphRenderParam,
    ) -> Option<&RenderGlyph> {
        let key = RenderGlyphKey {
            glyph_key,
            glyph_param,
        };

        self.retainer.insert(key.clone());

        use hash_map::Entry::*;
        match self.cache.entry(key) {
            Occupied(e) => e.into_mut().as_ref(),
            Vacant(e) => {
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
pub struct RenderGlyphKey {
    pub glyph_key: text::CacheKey,
    pub glyph_param: GlyphRenderParam,
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
    key: &RenderGlyphKey,
) -> Option<RenderGlyph> {
    let image = render_glyph_image(font_system, scale_context, key.glyph_key)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }

    if let Ok((placement, texture_view)) =
        image_to_gpu_texture(device, queue, &image, &key.glyph_param)
    {
        Some(RenderGlyph {
            placement,
            pipeline: key.glyph_param.pipeline(),
            texture_view,
        })
    } else {
        None
    }
}

fn image_to_gpu_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
    param: &GlyphRenderParam,
) -> Result<(text::Placement, texture::View)> {
    if param.sdf {
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
