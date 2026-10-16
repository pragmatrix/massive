use std::collections::HashMap;
use std::sync::{Arc, Weak};

use log::error;
use smallvec::SmallVec;

use wgpu::util::{BufferInitDescriptor, DeviceExt};

use massive_geometry::{SizePx, Vector3};
use massive_shapes::{Image, ImageSource, Shape};

use crate::pods::{self, AsBytes, TextureVertex, VertexLayout};
use crate::renderer::{BatchSegment, PreparationContext, RenderBatch};
use crate::tools::{PipelineParams, PipelineVariant, TextureBindGroupLayout, texture_sampler};

/// Draws [`Image`] shapes with one texture per image, so images neither evict glyphs from the
/// atlases nor run into their size limit.
///
/// Each image becomes its own batch segment, so one draw call.
#[derive(Debug)]
pub struct ImageRenderer {
    pipeline_params: PipelineParams,
    bind_group_layout: TextureBindGroupLayout,
    sampler: wgpu::Sampler,
    /// Keyed by the source's address.
    textures: HashMap<usize, ImageTexture>,
    /// Read once, because the device's limits are fixed for its lifetime.
    max_texture_dimension: u32,
}

impl ImageRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = TextureBindGroupLayout::new(device);
        // Shared with the color atlas, so images and color emoji blend alike.
        let shader = device.create_shader_module(wgpu::include_wgsl!("texture.wgsl"));
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
            max_texture_dimension: device.limits().max_texture_dimension_2d,
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
            .retain(|_, texture| texture.source.strong_count() > 0);

        let images = shapes.iter().filter_map(|shape| match shape {
            Shape::Image(image) => Some(image),
            _ => None,
        });

        let image_count = images.clone().count();
        let mut vertices = Vec::with_capacity(image_count * 4);
        let mut segments = SmallVec::with_capacity(image_count);
        for image in images {
            let Some(bind_group) = self.bind_group(context, image) else {
                continue;
            };
            let quad = segments.len();
            segments.push(BatchSegment {
                fs_bind_group: Some(bind_group),
                quads: quad..quad + 1,
            });
            vertices.extend(image_vertices(image));
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

    /// The bind group of the image's texture, uploading it the first time its source is drawn.
    /// `None` when the pixels do not match their size or exceed the device's texture size, which
    /// their producer cannot know.
    fn bind_group(
        &mut self,
        context: &PreparationContext,
        image: &Image,
    ) -> Option<wgpu::BindGroup> {
        let key = Arc::as_ptr(&image.source) as *const () as usize;
        if let Some(texture) = self.textures.get(&key) {
            return texture.bind_group.clone();
        }

        let bind_group = upload_texture(context, image.source.as_ref(), self.max_texture_dimension)
            .map(|view| {
                self.bind_group_layout
                    .create_bind_group(context.device, &view, &self.sampler)
            });
        self.textures.insert(
            key,
            ImageTexture {
                source: Arc::downgrade(&image.source),
                bind_group: bind_group.clone(),
            },
        );
        bind_group
    }
}

/// An uploaded image. Batches hold clones of the bind group, which keep the texture alive.
#[derive(Debug)]
struct ImageTexture {
    /// Keeps the source's allocation alive, so no other source can reuse the address this entry is
    /// keyed by.
    source: Weak<dyn ImageSource>,
    /// `None` when the upload failed, so it is neither retried nor reported again while the
    /// source is drawn.
    bind_group: Option<wgpu::BindGroup>,
}

fn image_vertices(image: &Image) -> [TextureVertex; 4] {
    let (r, s) = (&image.rect, &image.region);
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

fn upload_texture(
    context: &PreparationContext,
    source: &dyn ImageSource,
    max_dimension: u32,
) -> Option<wgpu::TextureView> {
    let mut view = None;
    source.read(&mut |size, rgba| view = upload_rgba(context, size, rgba, max_dimension));
    view
}

fn upload_rgba(
    context: &PreparationContext,
    size: SizePx,
    rgba: &[u8],
    max_dimension: u32,
) -> Option<wgpu::TextureView> {
    let expected_len = size.width as usize * size.height as usize * 4;
    if rgba.len() != expected_len {
        error!(
            "Image of {size:?} has {} RGBA8 bytes, expected {expected_len}",
            rgba.len()
        );
        return None;
    }
    if size.width > max_dimension || size.height > max_dimension {
        error!("Image of {size:?} exceeds the maximum texture dimension {max_dimension}");
        return None;
    }

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
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 4),
            rows_per_image: None,
        },
        extent,
    );
    Some(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}
