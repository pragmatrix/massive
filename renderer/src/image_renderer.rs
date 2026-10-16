use std::collections::HashMap;
use std::sync::{Arc, Weak};

use log::warn;
use smallvec::SmallVec;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use massive_geometry::Vector3;
use massive_shapes::{Image, ImagePixels, Shape};

use crate::pods::{self, AsBytes, VertexLayout};
use crate::renderer::{BatchSegment, PreparationContext, RenderBatch};
use crate::text_layer::{BindGroupLayout, TextureVertex};
use crate::tools::{PipelineParams, PipelineVariant, texture_sampler};

/// Draws [`Image`] shapes with one texture per image, so images neither evict glyphs from the
/// atlases nor run into their size limit.
///
/// Each run of images with the same pixels becomes one batch segment, so one draw call.
#[derive(Debug)]
pub struct ImageRenderer {
    pipeline_params: PipelineParams,
    bind_group_layout: BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Keyed by the pixels' address. The `Weak` keeps that allocation alive, so other pixels cannot
    /// reuse the address while the entry exists.
    textures: HashMap<usize, (Weak<ImagePixels>, wgpu::BindGroup)>,
}

impl ImageRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = BindGroupLayout::new(device);
        // The color atlas shader samples straight-alpha RGBA by pixel coordinates, which is what
        // images need too, and it keeps their colors consistent with color emoji.
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("text_layer/color_atlas.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Image Pipeline Layout"),
            bind_group_layouts: &[Some(&*bind_group_layout)],
            immediate_size: pods::Immediates::size(),
        });
        let targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        Self {
            pipeline_params: PipelineParams {
                shader,
                pipeline_layout,
                targets,
                vertex_layout: [TextureVertex::layout()],
            },
            bind_group_layout,
            sampler: texture_sampler::linear_clamping(device),
            textures: HashMap::new(),
        }
    }

    pub fn create_pipeline(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> wgpu::RenderPipeline {
        self.pipeline_params
            .create_pipeline("Image Pipeline", device, "fs_main", variant)
    }

    pub fn batch_from_shapes(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
    ) -> Option<RenderBatch> {
        // Pruned here because producers have no per-frame hook; the map holds a few entries.
        self.textures
            .retain(|_, (pixels, _)| pixels.strong_count() > 0);

        let images = shapes.iter().filter_map(|shape| match shape {
            Shape::Image(image) => Some(image),
            _ => None,
        });

        let mut vertices = Vec::new();
        let mut segments = SmallVec::new();
        // The pixels of the previous image and whether they have a texture, so a run of images
        // with the same pixels looks its texture up once.
        let mut previous: Option<(&Arc<ImagePixels>, bool)> = None;
        for image in images {
            let drawable = match previous {
                Some((pixels, drawable)) if Arc::ptr_eq(pixels, &image.pixels) => drawable,
                _ => {
                    let bind_group = self.bind_group(context, &image.pixels);
                    let drawable = bind_group.is_some();
                    if let Some(bind_group) = bind_group {
                        let start = vertices.len() / 4;
                        segments.push(BatchSegment {
                            fs_bind_group: Some(bind_group),
                            quads: start..start,
                        });
                    }
                    previous = Some((&image.pixels, drawable));
                    drawable
                }
            };
            if drawable {
                vertices.extend(image_vertices(image));
                let segment: &mut BatchSegment =
                    segments.last_mut().expect("A drawable image has a segment");
                segment.quads.end += 1;
            }
        }

        if segments.is_empty() {
            return None;
        }

        let vertex_buffer = context.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Image Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(RenderBatch {
            vertex_buffer,
            segments,
        })
    }

    /// The bind group of the pixels' texture, uploading it the first time. `None` when the image
    /// exceeds the device's texture size, which the caller cannot know.
    fn bind_group(
        &mut self,
        context: &PreparationContext,
        pixels: &Arc<ImagePixels>,
    ) -> Option<wgpu::BindGroup> {
        let key = Arc::as_ptr(pixels) as usize;
        if let Some((_, bind_group)) = self.textures.get(&key) {
            return Some(bind_group.clone());
        }

        let size = pixels.size();
        let max_dimension = context.device.limits().max_texture_dimension_2d;
        if size.width > max_dimension || size.height > max_dimension {
            warn!("Image of {size:?} exceeds the maximum texture dimension {max_dimension}");
            return None;
        }

        let view = upload_texture(context, pixels);
        let bind_group =
            self.bind_group_layout
                .create_bind_group(context.device, &view, &self.sampler);
        self.textures
            .insert(key, (Arc::downgrade(pixels), bind_group.clone()));
        Some(bind_group)
    }
}

fn image_vertices(image: &Image) -> [TextureVertex; 4] {
    let (r, s) = (&image.rect, &image.source);
    let vertex = |x: f64, y: f64, u: f64, v: f64| {
        TextureVertex::new(Vector3::new(x, y, 0.0), (u as f32, v as f32))
    };
    // Same corner order as the atlas quads, so the shared index buffer's winding applies.
    [
        vertex(r.left, r.top, s.left, s.top),
        vertex(r.left, r.bottom, s.left, s.bottom),
        vertex(r.right, r.bottom, s.right, s.bottom),
        vertex(r.right, r.top, s.right, s.top),
    ]
}

fn upload_texture(context: &PreparationContext, pixels: &ImagePixels) -> wgpu::TextureView {
    let size = pixels.size();
    let extent = wgpu::Extent3d {
        width: size.width,
        height: size.height,
        depth_or_array_layers: 1,
    };
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Image"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Matches the color atlas, so images and color emoji blend alike.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    context.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels.rgba(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 4),
            rows_per_image: None,
        },
        extent,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
