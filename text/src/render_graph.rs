use std::{mem, ops::DerefMut};

use cosmic_text::{self as text, SwashImage};
use granularity_geometry::Point;
use granularity_shell::{time, Shell};
use log::debug;
use swash::{
    scale::{image::Image, Render, ScaleContext, Source, StrikeWith},
    zeno::Format,
    FontRef,
};
use text::AttrsList;
use wgpu::util::DeviceExt;

use granularity::{map_ref, Value};
use granularity_geometry::{scalar, view_projection_matrix, Camera, Projection};

struct PlacedGlyph {
    cache_key: text::CacheKey,
    pos: (i32, i32),
}

impl PlacedGlyph {
    fn new(cache_key: text::CacheKey, pos: (i32, i32)) -> Self {
        Self { cache_key, pos }
    }
}

fn place_glyphs(glyphs: &[text::LayoutGlyph]) -> Vec<PlacedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            // TODO: disable Subpixel rendering?
            let (cc, x, y) = text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                (glyph.x, glyph.y),
            );
            PlacedGlyph::new(cc, (x, y))
        })
        .collect()
}

pub fn render_graph(
    camera: Value<Camera>,
    text: Value<String>,
    shell: &Shell,
) -> (Value<wgpu::CommandBuffer>, Value<wgpu::SurfaceTexture>) {
    let font_system = &shell.font_system;
    let glyph_cache = &shell.glyph_cache;
    let device = &shell.device;
    let queue = &shell.queue;
    let config = &shell.config;
    let surface = &shell.surface;

    let shader = map_ref!(|device| {
        device.create_shader_module(wgpu::include_wgsl!("shaders/character-shader.wgsl"))
    });

    // TODO: handle errors here (but how or if? should they propagate through the graph?)
    let output = map_ref!(|surface| surface.get_current_texture().unwrap());

    let view = map_ref!(|output| {
        output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    });

    let font_size = 140.0;

    // Text

    let placed_glyphs = map_ref!(|font_system, text| {
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
        let line = &buffer.layout(font_system, font_size, f32::MAX, text::Wrap::None)[0].glyphs;
        place_glyphs(line)
    });

    // TODO: cache these, too.
    let glyph_texture_views = map_ref!(|device, queue, font_system, glyph_cache, placed_glyphs| {
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
                        (image.placement.width != 0 && image.placement.height != 0).then_some(image)
                    })
                    .map(|image| image_to_texture_view(device, queue, image))
            })
            .collect::<Vec<_>>()
    });

    // Sample & Texture Bind Group

    let texture_sampler = map_ref!(|device| device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    }));

    let texture_bind_group_layout = map_ref!(|device| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    // This should match the filterable field of the
                    // corresponding Texture entry above.
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    });

    let texture_bind_groups = map_ref!(|device,
                                        texture_bind_group_layout,
                                        glyph_texture_views,
                                        texture_sampler| {
        glyph_texture_views
            .iter()
            .map(|texture_view| {
                texture_view.as_ref().map(|texture_view| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Texture Bind Group"),
                        layout: texture_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(texture_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(texture_sampler),
                            },
                        ],
                    })
                })
            })
            .collect::<Vec<_>>()
    });

    // Camera

    let projection = map_ref!(|config| Projection::new(
        config.width as scalar / config.height as scalar,
        0.1,
        100.0
    ));

    let view_projection_uniform = map_ref!(|camera, projection| {
        let matrix = view_projection_matrix(camera, projection);
        let m: cgmath::Matrix4<f32> = matrix.cast().expect("matrix casting to f32 failed");
        ViewProjectionUniform(m.into())
    });

    let view_projection_buffer = map_ref!(|device, view_projection_uniform| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[*view_projection_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let camera_bind_group_layout = map_ref!(|device| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("Camera Bind Group Layout"),
        })
    });

    let camera_bind_group =
        map_ref!(
            |device, camera_bind_group_layout, view_projection_buffer| device.create_bind_group(
                &wgpu::BindGroupDescriptor {
                    layout: camera_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: view_projection_buffer.as_entire_binding(),
                    }],
                    label: Some("Camera Bind Group"),
                }
            )
        );

    // Pipeline

    let render_pipeline_layout = map_ref!(
        |device, texture_bind_group_layout, camera_bind_group_layout| device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[texture_bind_group_layout, camera_bind_group_layout],
                push_constant_ranges: &[],
            })
    );

    let targets = map_ref!(|config| [Some(wgpu::ColorTargetState {
        format: config.format,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
    })]);

    let pipeline = map_ref!(|device, shader, render_pipeline_layout, targets| {
        let pipeline = wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: "vs_main",
                buffers: &[TextureVertex::desc().clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: "fs_main",
                targets,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        };

        device.create_render_pipeline(&pipeline)
    });

    const SZ: f32 = 1.0;

    // Vertex Buffer (must live longer than render_pass)
    const VERTICES: &[TextureVertex] = &[
        TextureVertex {
            position: [-SZ, SZ, 0.0],
            tex_coords: [0.0, 0.0],
        },
        TextureVertex {
            position: [-SZ, -SZ, 0.0],
            tex_coords: [0.0, 1.0],
        },
        TextureVertex {
            position: [SZ, -SZ, 0.0],
            tex_coords: [1.0, 1.0],
        },
        TextureVertex {
            position: [SZ, SZ, 0.0],
            tex_coords: [1.0, 0.0],
        },
    ];

    let vertex_buffer = map_ref!(|device| device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        }
    ));

    const INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

    let index_buffer = map_ref!(|device| device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        }
    ));

    let command_buffer = map_ref!(|device,
                                   view,
                                   pipeline,
                                   texture_bind_groups,
                                   camera_bind_group,
                                   vertex_buffer,
                                   index_buffer| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(1, camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16); // 1.

            for texture_bind_group in texture_bind_groups.iter().filter_map(|b| b.as_ref()) {
                render_pass.set_bind_group(0, texture_bind_group, &[]);
                render_pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
            }
        }
        encoder.finish()
    });

    (command_buffer, output)
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
}

impl Vertex {
    fn desc() -> &'static wgpu::VertexBufferLayout<'static> {
        const LAYOUT: wgpu::VertexBufferLayout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        };

        &LAYOUT
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TextureVertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

impl TextureVertex {
    fn desc() -> &'static wgpu::VertexBufferLayout<'static> {
        const LAYOUT: wgpu::VertexBufferLayout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<TextureVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
        };

        &LAYOUT
    }
}

// We need this for Rust to store our data correctly for the shaders
#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewProjectionUniform([[f32; 4]; 4]);

// Render a character using swash.

fn render_character(c: char) -> Image {
    let mut context = ScaleContext::new();
    let font = include_bytes!("fonts/Roboto-Regular.ttf");
    let font = FontRef::from_index(font, 0).unwrap();

    let scaler_builder = context.builder(font);
    let mut scaler = scaler_builder.size(200.0).hint(false).build();

    let glyph_id = font.charmap().map(c);

    // We don't really care how the final image is rendered in detail, so we initialize a priority
    // list of sources and let the renderer decide what to use.
    let mut render = Render::new(&[
        // Color outline with the first palette
        Source::ColorOutline(0),
        // Color bitmap with best fit selection mode
        Source::ColorBitmap(StrikeWith::BestFit),
        // Standard scalable outline
        Source::Outline,
    ]);
    render.format(Format::Alpha).offset((0.0, 0.0).into());

    render.render(&mut scaler, glyph_id).expect("image")
}

/// Creates an empty texture and queues it for uploading to the GPU.
fn image_to_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &SwashImage,
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

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    texture_view
}
