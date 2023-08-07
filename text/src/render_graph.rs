use derive_more::Constructor;
use wgpu::util::DeviceExt;

use granularity::{map, Value};
use granularity_geometry::{scalar, view_projection_matrix, Bounds3, Camera, Matrix4, Projection};
// use granularity_shell::Shell;

// use crate::{layout, new_label, TextureVertex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pipeline {
    Flat,
    Sdf,
}

#[derive(Debug, Constructor)]
pub struct PipelineTextureView {
    pipeline: Pipeline,
    texture_view: wgpu::TextureView,
    size: (u32, u32),
}

#[derive(Debug, Constructor)]
pub struct PipelineBindGroup {
    pub pipeline: Pipeline,
    pub bind_group: wgpu::BindGroup,
}

pub fn render_graph(
    camera: Value<Camera>,
    text: Value<String>,
    shell: &Shell,
) -> (Value<wgpu::CommandBuffer>, Value<wgpu::SurfaceTexture>) {
    let device = &shell.device;
    let config = &shell.surface_config;
    let surface = &shell.surface;
    let surface_config = &shell.surface_config;

    // Create a pixel bounds for a window that covers the entire surface.
    let window_bounds = map!(|surface_config| {
        let half_width = surface_config.width / 2;
        let half_height = surface_config.height / 2;
        Bounds3::new(
            (-(half_width as f64), -(half_height as f64), 0.0),
            (half_width as f64, half_height as f64, 0.0),
        )
    });

    let shader = map!(|device| {
        device.create_shader_module(wgpu::include_wgsl!("shaders/character-shader.wgsl"))
    });

    // TODO: handle errors here (but how or if? should they propagate through the graph?)
    let output = map!(|surface| surface.get_current_texture().unwrap());

    let view = map!(|output| {
        output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    });

    // Camera

    let projection = map!(|config| Projection::new(
        config.width as scalar / config.height as scalar,
        0.1,
        100.0
    ));

    let view_projection_matrix =
        map!(|camera, projection| view_projection_matrix(camera, projection));

    // Label

    let font_size = shell.surface.runtime().var(100.0);

    let label = new_label(shell, font_size, text);

    // A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    // 1.0 in each axis. Also flips y.
    let pixel_matrix = map!(|surface_config| {
        Matrix4::from_nonuniform_scale(1.0, -1.0, 1.0)
            * Matrix4::from_scale(1.0 / surface_config.height as f64 * 2.0)
    });

    // A Matrix that translates from the WGPU coordinate system to the surface coordinate
    let surface_matrix = map!(|surface_config| {
        println!("surface_config: {:?}", surface_config);

        Matrix4::from_nonuniform_scale(
            surface_config.width as f64 / 2.0,
            (surface_config.height as f64 / 2.0) * -1.0,
            1.0,
        ) * Matrix4::from_translation(cgmath::Vector3::new(1.0, -1.0, 0.0))
    });

    let center_matrix = layout::center(window_bounds, label.metrics.clone());

    // let label_matrix = map!(|pixel_matrix, center_matrix| pixel_matrix * center_matrix);
    let label_matrix = map!(|pixel_matrix, center_matrix, view_projection_matrix| {
        view_projection_matrix * pixel_matrix * center_matrix
    });

    let label = label.render(shell, label_matrix.clone(), surface_matrix.clone());

    // Sampler & Texture Bind Group

    let texture_sampler = map!(|device| device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    }));

    let texture_bind_group_layout = map!(|device| {
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
                // Texture size
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    // This should match the filterable field of the
                    // corresponding Texture entry above.
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    });

    let placements_and_texture_views = &label.placements_and_texture_views;

    let texture_bind_groups = map!(|device,
                                    texture_bind_group_layout,
                                    placements_and_texture_views,
                                    texture_sampler| {
        placements_and_texture_views
            .iter()
            .enumerate()
            .map(|(_, placement_and_view)| {
                placement_and_view.as_ref().map(|(_, texture_view)| {
                    let texture_size = texture_view.size;
                    let texture_size =
                        TextureSizeUniform([texture_size.0 as f32, texture_size.1 as f32]);
                    let texture_size_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Texture Size Buffer"),
                            contents: bytemuck::cast_slice(&[texture_size]),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        });

                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Texture Bind Group"),
                        layout: texture_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(
                                    &texture_view.texture_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: texture_size_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(texture_sampler),
                            },
                        ],
                    });
                    PipelineBindGroup {
                        pipeline: texture_view.pipeline,
                        bind_group,
                    }
                })
            })
            .collect::<Vec<_>>()
    });

    // Model Matrix

    let model_matrix_uniform = map!(|label_matrix| {
        let m: cgmath::Matrix4<f32> = label_matrix.cast().expect("matrix casting to f32 failed");
        Matrix4Uniform(m.into())
    });

    let model_matrix_buffer = map!(|device, model_matrix_uniform| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model Matrix Buffer"),
            contents: bytemuck::cast_slice(&[*model_matrix_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let model_matrix_bind_group_layout = map!(|device| {
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
            label: Some("Model Matrix Bind Group Layout"),
        })
    });

    let model_matrix_bind_group = map!(
        |device, model_matrix_bind_group_layout, model_matrix_buffer| device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                layout: model_matrix_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: model_matrix_buffer.as_entire_binding(),
                }],
                label: Some("Model Matrix Bind Group"),
            }
        )
    );

    // Pipeline

    let render_pipeline_layout = map!(
        |device, texture_bind_group_layout, model_matrix_bind_group_layout| device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[model_matrix_bind_group_layout, texture_bind_group_layout],
                push_constant_ranges: &[],
            })
    );

    let targets = map!(|config| [Some(wgpu::ColorTargetState {
        format: config.format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    })]);

    let pipelines = map!(|device, shader, render_pipeline_layout, targets| {
        [
            (
                Pipeline::Flat,
                create_pipeline(
                    "Pipeline",
                    device,
                    shader,
                    render_pipeline_layout,
                    targets,
                    "fs_flat",
                ),
            ),
            (
                Pipeline::Sdf,
                create_pipeline(
                    "SDF Pipeline",
                    device,
                    shader,
                    render_pipeline_layout,
                    targets,
                    "fs_sdf",
                ),
            ),
        ]
    });

    const INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

    let index_buffer = map!(|device| device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        }
    ));

    let vertex_buffers = &label.vertex_buffers;

    let command_buffer = map!(|device,
                               view,
                               pipelines,
                               texture_bind_groups,
                               model_matrix_bind_group,
                               vertex_buffers,
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

            for pipeline in pipelines {
                let kind = pipeline.0;
                let pipeline = &pipeline.1;
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, model_matrix_bind_group, &[]);
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16); // 1.

                for (i, texture_bind_group) in texture_bind_groups
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
                {
                    if texture_bind_group.pipeline != kind {
                        continue;
                    }
                    render_pass.set_bind_group(1, &texture_bind_group.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, vertex_buffers[i].as_ref().unwrap().slice(..));
                    render_pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
                }
            }
        }
        encoder.finish()
    });

    (command_buffer, output)
}

// We need this for Rust to store our data correctly for the shaders
#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Matrix4Uniform([[f32; 4]; 4]);

#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TextureSizeUniform([f32; 2]);

fn create_pipeline(
    label: &str,
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    render_pipeline_layout: &wgpu::PipelineLayout,
    targets: &[Option<wgpu::ColorTargetState>],
    fragment_shader_entry: &str,
) -> wgpu::RenderPipeline {
    let pipeline = wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &[TextureVertex::desc().clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: fragment_shader_entry,
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
}
