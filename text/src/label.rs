use std::ops::DerefMut;

use cgmath::Point2;
use cosmic_text as text;
use granularity::{map_ref, Value};
use granularity_shell::Shell;
use wgpu::{util::DeviceExt, TextureView};

use crate::TextureVertex;

pub struct Label {
    pub placed_glyphs: Value<Vec<PlacedGlyph>>,
    /// TODO: Separate?
    pub placements_and_texture_views: Value<Vec<Option<(text::Placement, TextureView)>>>,
    pub vertex_buffers: Value<Vec<Option<wgpu::Buffer>>>,
}

pub fn new_label(shell: &Shell, font_size: Value<f32>, text: Value<String>) -> Label {
    let device = &shell.device;
    let queue = &shell.queue;
    let font_system = &shell.font_system;
    let glyph_cache = &shell.glyph_cache;
    let surface_config = &shell.surface_config;

    let placed_glyphs = map_ref!(|font_system, text, font_size| {
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
        let line = &buffer.layout(font_system, *font_size, f32::MAX, text::Wrap::None)[0].glyphs;
        place_glyphs(line)
    });

    // For now they have to be combined because we only receive placements and the imagines together
    // from the SwashCache, and the images are only accessible by reference.
    // TODO: Find a way to separate them.
    let placements_and_texture_views =
        map_ref!(|device, queue, font_system, glyph_cache, placed_glyphs| {
            let mut font_system = font_system.borrow_mut();
            let mut glyph_cache = glyph_cache.borrow_mut();
            let glyph_cache = glyph_cache.deref_mut();
            placed_glyphs
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
        });

    let vertex_buffers = map_ref!(
        |device, surface_config, placed_glyphs, placements_and_texture_views| {
            placements_and_texture_views
                .iter()
                .enumerate()
                .map(|(i, placement_and_view)| {
                    placement_and_view.as_ref().map(|(placement, _)| {
                        let rect = place_glyph(placed_glyphs[i].pos, *placement);

                        let vertices = glyph_to_texture_vertex(
                            surface_config,
                            (rect.0.cast().unwrap(), rect.1.cast().unwrap()),
                        );

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
        placed_glyphs,
        placements_and_texture_views,
        vertex_buffers,
    }
}

pub struct PlacedGlyph {
    pub cache_key: text::CacheKey,
    pub pos: (i32, i32),
}

impl PlacedGlyph {
    fn new(cache_key: text::CacheKey, pos: (i32, i32)) -> Self {
        Self { cache_key, pos }
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

            // TODO: disable Subpixel rendering?
            let (cc, x, y) = text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
            );
            PlacedGlyph::new(cc, (x, y))
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

// Until vertex conversion, coordinate system is ((0,0), (surface.width,surface.height))
const BASELINE_Y: i32 = 200;

// TODO: need a rect structure.

fn place_glyph(pos: (i32, i32), placement: text::Placement) -> (Point2<i32>, Point2<i32>) {
    let left = pos.0 + placement.left;
    // placement goes up (right handed coordinate system).
    let top = pos.1 + BASELINE_Y - placement.top;
    let right = left + placement.width as i32;
    let bottom = top + placement.height as i32;

    ((left, top).into(), (right, bottom).into())
}

fn glyph_to_texture_vertex(
    surface_config: &wgpu::SurfaceConfiguration,
    rect: (Point2<f32>, Point2<f32>),
) -> [TextureVertex; 4] {
    // TODO: use a 2D matrix here?
    let left = rect.0.x / surface_config.height as f32 * 2.0 - 1.0;
    let top = (rect.0.y / surface_config.height as f32 * 2.0 - 1.0) * -1.0;
    let right = rect.1.x / surface_config.height as f32 * 2.0 - 1.0;
    let bottom = (rect.1.y / surface_config.height as f32 * 2.0 - 1.0) * -1.0;

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
