use std::ops::DerefMut;

use cgmath::Point2;
use cosmic_text as text;
use granularity::{map_ref, Value};
use granularity_geometry::Size3;
use granularity_shell::Shell;
use wgpu::util::DeviceExt;

use crate::{Extent, TextureVertex};

pub struct Label {
    placed_glyphs: Value<(LabelMetrics, Vec<PlacedGlyph>)>,
    pub metrics: Value<LabelMetrics>,
    /// TODO: Separate?
    pub placements_and_texture_views: Value<Vec<Option<(text::Placement, wgpu::TextureView)>>>,
    pub vertex_buffers: Value<Vec<Option<wgpu::Buffer>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LabelMetrics {
    pub max_ascent: u32,
    pub max_descent: u32,
    pub width: u32,
}

impl LabelMetrics {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.max_ascent + self.max_descent)
    }
}

impl Extent for LabelMetrics {
    fn size(&self) -> Size3 {
        let (width, height) = self.size();
        Size3::new((width as f64, height as f64, 0.0).into())
    }

    fn baseline(&self) -> Option<f64> {
        Some(self.max_ascent as f64)
    }
}

pub fn new_label(shell: &Shell, font_size: Value<f32>, text: Value<String>) -> Label {
    let device = &shell.device;
    let queue = &shell.queue;
    let font_system = &shell.font_system;
    let glyph_cache = &shell.glyph_cache;

    let metrics_and_placed_glyphs = map_ref!(|font_system, text, font_size| {
        let mut font_system = font_system.borrow_mut();
        let font_system = font_system.deref_mut();
        // TODO: Cosmic text recommends to use a single buffer for a widget, but we are creating a
        // new one every time the text change. Not sure if that makes a big difference, because it
        // seems that all the shaping information is being destroyed and only the buffer's memory
        // is preserved.
        let mut buffer = text::BufferLine::new(
            text,
            text::AttrsList::new(text::Attrs::new()),
            text::Shaping::Advanced,
        );
        let line = &buffer.layout(font_system, *font_size, f32::MAX, text::Wrap::None)[0];
        let line_glyphs = &line.glyphs;
        let placed = place_glyphs(line_glyphs);
        let metrics = LabelMetrics {
            max_ascent: line.max_ascent as u32,
            max_descent: line.max_descent as u32,
            width: line.w.ceil() as u32,
        };
        (metrics, placed)
    });

    let metrics = map_ref!(|metrics_and_placed_glyphs| metrics_and_placed_glyphs.0);

    // For now they have to be combined because we only receive placements and the imagines together
    // from the SwashCache, and the images are only accessible by reference.
    // TODO: Find a way to separate them.
    let placements_and_texture_views = map_ref!(
        |device, queue, font_system, glyph_cache, metrics_and_placed_glyphs| {
            let mut font_system = font_system.borrow_mut();
            let mut glyph_cache = glyph_cache.borrow_mut();
            let glyph_cache = glyph_cache.deref_mut();
            let metrics = &metrics_and_placed_glyphs.0;
            metrics_and_placed_glyphs
                .1
                .iter()
                .map(|placed_glyph| {
                    let image = glyph_cache
                        .get_image(&mut font_system, placed_glyph.cache_key)
                        .as_ref();

                    image
                        .and_then(|image| {
                            (image.placement.width != 0 && image.placement.height != 0)
                                .then_some(image)
                        })
                        .map(|image| (image.placement, image_to_texture(device, queue, image)))
                })
                .collect::<Vec<_>>()
        }
    );

    let vertex_buffers = map_ref!(
        |device, metrics_and_placed_glyphs, placements_and_texture_views| {
            let metrics = &metrics_and_placed_glyphs.0;
            placements_and_texture_views
                .iter()
                .enumerate()
                .map(|(i, placement_and_view)| {
                    placement_and_view.as_ref().map(|(placement, _)| {
                        let rect = place_glyph(
                            metrics.max_ascent,
                            metrics_and_placed_glyphs.1[i].hitbox_pos,
                            *placement,
                        );

                        let vertices = glyph_to_texture_vertex((
                            rect.0.cast().unwrap(),
                            rect.1.cast().unwrap(),
                        ));

                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Vertex Buffer"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        })
                    })
                })
                .collect::<Vec<_>>()
        }
    );

    Label {
        placed_glyphs: metrics_and_placed_glyphs,
        metrics,
        placements_and_texture_views,
        vertex_buffers,
    }
}

#[derive(Debug)]
pub struct PlacedGlyph {
    pub cache_key: text::CacheKey,
    pub hitbox_pos: (i32, i32),
    pub hitbox_width: f32,
}

impl PlacedGlyph {
    fn new(cache_key: text::CacheKey, hitbox_pos: (i32, i32), hitbox_width: f32) -> Self {
        Self {
            cache_key,
            hitbox_pos,
            hitbox_width,
        }
    }
}

const RENDER_SUBPIXEL: bool = false;

fn place_glyphs(glyphs: &[text::LayoutGlyph]) -> Vec<PlacedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            let fractional_pos = if RENDER_SUBPIXEL {
                (glyph.x, glyph.y)
            } else {
                (glyph.x.round(), glyph.y.round())
            };

            let (cc, x, y) = text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
            );
            // Note: hitbox with is fractional, but does not change with / without subpixel
            // rendering.
            PlacedGlyph::new(cc, (x, y), glyph.w)
        })
        .collect()
}

/// Creates an empty texture and queues it for uploading to the GPU.
fn image_to_texture(
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
            // TODO: this looks optional.
            rows_per_image: Some(image.placement.height),
        },
        texture_size,
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

// TODO: need a rect structure.

fn place_glyph(
    max_ascent: u32,
    hitbox_pos: (i32, i32),
    placement: text::Placement,
) -> (Point2<i32>, Point2<i32>) {
    let left = hitbox_pos.0 + placement.left;
    let top = hitbox_pos.1 + (max_ascent as i32) - placement.top;
    let right = left + placement.width as i32;
    let bottom = top + placement.height as i32;

    ((left, top).into(), (right, bottom).into())
}

fn glyph_to_texture_vertex(rect: (Point2<f32>, Point2<f32>)) -> [TextureVertex; 4] {
    let left = rect.0.x;
    let top = rect.0.y;
    let right = rect.1.x;
    let bottom = rect.1.y;

    [
        TextureVertex {
            position: (left, top, 0.0).into(),
            tex_coords: [0.0, 0.0],
        },
        TextureVertex {
            position: (left, bottom, 0.0).into(),
            tex_coords: [0.0, 1.0],
        },
        TextureVertex {
            position: (right, bottom, 0.0).into(),
            tex_coords: [1.0, 1.0],
        },
        TextureVertex {
            position: (right, top, 0.0).into(),
            tex_coords: [1.0, 0.0],
        },
    ]
}
