use cgmath::SquareMatrix;
use wgpu::util::DeviceExt;

use granularity::{map_ref, Value};
use granularity_geometry::{scalar, view_projection_matrix, Bounds3, Camera, Matrix4, Projection};
use granularity_shell::Shell;

use crate::{layout, new_label, TextureVertex};

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
    let window_bounds = map_ref!(|surface_config| {
        let half_width = surface_config.width / 2;
        let half_height = surface_config.height / 2;
        Bounds3::new(
            (-(half_width as f64), -(half_height as f64), 0.0),
            (half_width as f64, half_height as f64, 0.0),
        )
    });

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

    // Camera

    let projection = map_ref!(|config| Projection::new(
        config.width as scalar / config.height as scalar,
        0.1,
        100.0
    ));

    let view_projection_matrix =
        map_ref!(|camera, projection| view_projection_matrix(camera, projection));

    let view_projection_uniform = map_ref!(|view_projection_matrix| {
        let m: cgmath::Matrix4<f32> = view_projection_matrix
            .cast()
            .expect("matrix casting to f32 failed");
        Matrix4Uniform(m.into())
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

    // Label

    let font_size = shell.surface.runtime().var(280.0);

    let label = new_label(shell, font_size, text);

    // A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    // 1.0 in each axis. Also flips y.
    let pixel_matrix = map_ref!(|surface_config| {
        Matrix4::from_nonuniform_scale(1.0, -1.0, 1.0)
            * Matrix4::from_scale(1.0 / surface_config.height as f64 * 2.0)
    });

    // A Matrix that translates from the WGPU coordinate system to the surface coordinate
    let surface_matrix = map_ref!(|surface_config| {
        println!("surface_config: {:?}", surface_config);

        Matrix4::from_nonuniform_scale(
            surface_config.width as f64 / 2.0,
            (surface_config.height as f64 / 2.0) * -1.0,
            1.0,
        ) * Matrix4::from_translation(cgmath::Vector3::new(1.0, -1.0, 0.0))
    });

    let center_matrix = layout::center(window_bounds, label.metrics.clone());

    // let label_matrix = map_ref!(|pixel_matrix, center_matrix| pixel_matrix * center_matrix);
    let label_matrix = map_ref!(|pixel_matrix, center_matrix, view_projection_matrix| {
        view_projection_matrix * pixel_matrix * center_matrix
    });

    let label = label.render(shell, label_matrix.clone(), surface_matrix.clone());

    // Sample & Texture Bind Group

    let texture_sampler = map_ref!(|device| device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
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

    let placements_and_texture_views = &label.placements_and_texture_views;

    let texture_bind_groups = map_ref!(|device,
                                        texture_bind_group_layout,
                                        placements_and_texture_views,
                                        texture_sampler| {
        placements_and_texture_views
            .iter()
            .enumerate()
            .map(|(_, placement_and_view)| {
                placement_and_view.as_ref().map(|(_, texture_view)| {
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

    // Model Matrix

    let model_matrix_uniform = map_ref!(|label_matrix| {
        let m: cgmath::Matrix4<f32> = label_matrix.cast().expect("matrix casting to f32 failed");
        Matrix4Uniform(m.into())
    });

    let model_matrix_buffer = map_ref!(|device, model_matrix_uniform| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model Matrix Buffer"),
            contents: bytemuck::cast_slice(&[*model_matrix_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let model_matrix_bind_group_layout = map_ref!(|device| {
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

    let model_matrix_bind_group = map_ref!(
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

    let render_pipeline_layout =
        map_ref!(|device,
                  texture_bind_group_layout,
                  camera_bind_group_layout,
                  model_matrix_bind_group_layout| device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[
                    texture_bind_group_layout,
                    camera_bind_group_layout,
                    model_matrix_bind_group_layout
                ],
                push_constant_ranges: &[],
            }
        ));

    let targets = map_ref!(|config| [Some(wgpu::ColorTargetState {
        format: config.format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

    const INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

    let index_buffer = map_ref!(|device| device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        }
    ));

    let vertex_buffers = &label.vertex_buffers;

    let command_buffer = map_ref!(|device,
                                   view,
                                   pipeline,
                                   texture_bind_groups,
                                   camera_bind_group,
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

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(1, camera_bind_group, &[]);
            render_pass.set_bind_group(2, model_matrix_bind_group, &[]);
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16); // 1.

            for (i, texture_bind_group) in texture_bind_groups
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
            {
                render_pass.set_bind_group(0, texture_bind_group, &[]);
                render_pass.set_vertex_buffer(0, vertex_buffers[i].as_ref().unwrap().slice(..));
                render_pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
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
