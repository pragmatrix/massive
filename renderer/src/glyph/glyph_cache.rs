use std::collections::{hash_map, HashMap};

use anyhow::Result;
use cosmic_text as text;
use swash::scale::ScaleContext;

use crate::{
    command::{Pipeline, PipelineTextureView},
    glyph::{
        glyph_classifier::GlyphClass,
        glyph_renderer::{pad_image, render_glyph_image},
    },
};

use super::glyph_renderer::render_sdf;

pub struct GlyphCache {
    scaler: ScaleContext,
    cache: HashMap<RenderGlyphKey, Option<RenderGlyph>>,
}

impl GlyphCache {
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut text::FontSystem,
        key: RenderGlyphKey,
    ) -> Option<&RenderGlyph> {
        use hash_map::Entry::*;
        match self.cache.entry(key) {
            Occupied(e) => e.into_mut().as_ref(),
            Vacant(e) => {
                let glyph = render_glyph(device, queue, font_system, &mut self.scaler, e.key());
                e.insert(glyph).as_ref()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderGlyphKey {
    pub glyph_key: text::CacheKey,
    pub texture_param: TextureParam,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureParam {
    // TODO: Add scaling
    sdf: bool,
}

impl TextureParam {
    fn pipeline(&self) -> Pipeline {
        if self.sdf {
            Pipeline::Sdf
        } else {
            Pipeline::Flat
        }
    }
}

#[derive(Debug)]
pub struct RenderGlyph {
    placement: text::Placement,
    texture_view: PipelineTextureView,
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
        image_to_texture(device, queue, &image, &key.texture_param)
    {
        Some(RenderGlyph {
            placement,
            texture_view,
        })
    } else {
        None
    }
}

fn classification_to_param(class: GlyphClass) -> TextureParam {
    use GlyphClass::*;
    match class {
        Zoomed(_) | PixelPerfect { .. } => TextureParam { sdf: false },
        Distorted(_) => TextureParam { sdf: true },
    }
}

fn image_to_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
    param: &TextureParam,
) -> Result<(text::Placement, PipelineTextureView)> {
    match param.sdf {
        false => {
            let padded = pad_image(image);
            Ok((
                padded.placement,
                PipelineTextureView::new(
                    Pipeline::Flat,
                    create_gpu_texture(device, queue, &padded),
                    (padded.placement.width, padded.placement.height),
                ),
            ))
        }
        true => render_sdf(image)
            .map(|sdf_image| {
                (sdf_image.placement, {
                    PipelineTextureView::new(
                        Pipeline::Sdf,
                        create_gpu_texture(device, queue, &sdf_image),
                        (sdf_image.placement.width, sdf_image.placement.height),
                    )
                })
            })
            .ok_or_else(|| anyhow::anyhow!("Failed to generate SDF image")),
    }
}

/// Creates a texture and uploads the image's content to the GPU.
fn create_gpu_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
) -> wgpu::TextureView {
    let texture_size = wgpu::Extent3d {
        width: image.placement.width,
        height: image.placement.height,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        label: Some("Character Texture"),
        view_formats: &[],
    });

    // TODO: how to separate this from texture creation?
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
            bytes_per_row: Some(image.placement.width),
            rows_per_image: None,
        },
        texture_size,
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
