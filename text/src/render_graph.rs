use wgpu::util::DeviceExt;

use granularity::{map_ref, Value};
use granularity_geometry::{scalar, view_projection_matrix, Camera, Projection};
use granularity_shell::Shell;

use crate::{new_label, TextureVertex};

pub fn render_graph(
    camera: Value<Camera>,
    text: Value<String>,
    shell: &Shell,
) -> (Value<wgpu::CommandBuffer>, Value<wgpu::SurfaceTexture>) {
    let device = &shell.device;
    let config = &shell.surface_config;
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

    let font_size = shell.surface.runtime().var(28.0);

    let label = new_label(shell, font_size, text);

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
                // TODO: do we really need / use triangle lists.
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
struct ViewProjectionUniform([[f32; 4]; 4]);
