use std::mem;

use anyhow::Result;
use granularity_geometry::{scalar, view_projection_matrix, Camera, Projection};
use wgpu::util::DeviceExt;

use crate::{
    command::{Command, Pipeline},
    pods::{self, TextureVertex},
    texture::{self, Texture},
};

struct Glyph {}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface,
    surface_config: wgpu::SurfaceConfiguration,

    view_projection_buffer: wgpu::Buffer,
    view_projection_bind_group: wgpu::BindGroup,
    texture_sampler: wgpu::Sampler,
    texture_bind_group_layout: texture::BindGroupLayout,

    pipelines: [(Pipeline, wgpu::RenderPipeline); 2],

    quad_index_buffer: wgpu::Buffer,
}

impl Renderer {
    /// Creates a new renderer and reconfigures the surface according to the given configuration.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface,
        surface_config: wgpu::SurfaceConfiguration,
    ) -> Self {
        let view_projection_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("View Projection Matrix Buffer"),
            size: mem::size_of::<pods::Matrix4>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (view_projection_bind_group_layout, view_projection_bind_group) =
            create_view_projection_bind_group(&device, &view_projection_buffer);

        let texture_sampler = create_texture_sampler(&device);

        let texture_bind_group_layout = texture::BindGroupLayout::new(&device);

        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(Self::QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let pipelines = {
            let render_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Render Pipeline Layout"),
                    bind_group_layouts: &[
                        &view_projection_bind_group_layout,
                        &texture_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
                });

            let shader =
                device.create_shader_module(wgpu::include_wgsl!("shaders/character-shader.wgsl"));

            let targets = [Some(wgpu::ColorTargetState {
                format: surface_config.format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })];

            create_pipelines(&device, &shader, &render_pipeline_layout, &targets)
        };

        let mut renderer = Self {
            device,
            queue,
            surface,
            surface_config,
            view_projection_buffer,
            view_projection_bind_group,
            texture_sampler,
            texture_bind_group_layout,
            pipelines,

            quad_index_buffer,
        };

        renderer.reconfigure_surface();
        renderer
    }

    fn render_and_present_frame(&mut self, camera: Camera, commands: &[Command]) -> Result<()> {
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.queue_view_projection_matrix(camera);

        let command_buffer = {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: true,
                        },
                    })],
                    depth_stencil_attachment: None,
                });

                // let texture_bind_groups = Vec::new();
                for command in commands {
                    match command {
                        Command::DrawImage(pipeline, points, image_data) => {
                            let texture = Texture::from_vertices_and_image_data(
                                &self.device,
                                &self.queue,
                                *pipeline,
                                &self.texture_bind_group_layout,
                                points,
                                image_data,
                                &self.texture_sampler,
                            );
                        }
                        Command::DrawImageLazy(_, _, _) => todo!(),
                    }
                }

                // for pipeline in self.pipelines {
                //     let kind = pipeline.0;
                //     let pipeline = &pipeline.1;
                //     render_pass.set_pipeline(pipeline);
                //     render_pass.set_bind_group(0, &self.view_projection_bind_group, &[]);
                //     render_pass.set_index_buffer(
                //         self.quad_index_buffer.slice(..),
                //         wgpu::IndexFormat::Uint16,
                //     );

                //     for (i, texture_bind_group) in texture_bind_groups
                //         .iter()
                //         .enumerate()
                //         .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
                //     {
                //         if texture_bind_group.pipeline != kind {
                //             continue;
                //         }
                //         render_pass.set_bind_group(1, &texture_bind_group.bind_group, &[]);
                //         render_pass
                //             .set_vertex_buffer(0, vertex_buffers[i].as_ref().unwrap().slice(..));
                //         render_pass.draw_indexed(0..Self::QUAD_INDICES.len() as u32, 0, 0..1);
                //     }
                // }
            }
            encoder.finish()
        };

        self.queue.submit([command_buffer]);
        surface_texture.present();
        Ok(())
    }

    const QUAD_INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

    fn queue_view_projection_matrix(&self, camera: Camera) {
        let projection = Projection::new(
            self.surface_config.width as scalar / self.surface_config.height as scalar,
            0.1,
            100.0,
        );

        let view_projection_matrix = view_projection_matrix(&camera, &projection);

        let view_projection_uniform = {
            let m: cgmath::Matrix4<f32> = view_projection_matrix
                .cast()
                .expect("matrix casting to f32 failed");
            pods::Matrix4(m.into())
        };

        self.queue.write_buffer(
            &self.view_projection_buffer,
            0,
            bytemuck::cast_slice(&[view_projection_uniform]),
        )
    }

    /// Resizes the surface, if necessary.
    /// Keeps the surface size at least 1x1.
    fn resize_surface(&mut self, new_size: (u32, u32)) {
        let new_surface_size = (new_size.0.max(1), new_size.1.max(1));

        if new_surface_size == self.surface_size() {
            return;
        }
        let config = &mut self.surface_config;
        config.width = new_surface_size.0;
        config.height = new_surface_size.1;

        self.reconfigure_surface();
    }

    /// Returns the current surface size.
    /// It may not match the window's size, for example if the window's size is 0,0.
    fn surface_size(&self) -> (u32, u32) {
        let config = &self.surface_config;
        (config.width, config.height)
    }

    fn reconfigure_surface(&mut self) {
        self.surface.configure(&self.device, &self.surface_config)
    }
}

fn create_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    targets: &[Option<wgpu::ColorTargetState>],
) -> [(Pipeline, wgpu::RenderPipeline); 2] {
    [
        (
            Pipeline::Flat,
            create_pipeline("Pipeline", device, shader, layout, targets, "fs_flat"),
        ),
        (
            Pipeline::Sdf,
            create_pipeline("SDF Pipeline", device, shader, layout, targets, "fs_sdf"),
        ),
    ]
}

fn create_view_projection_bind_group(
    device: &wgpu::Device,
    view_projection_buffer: &wgpu::Buffer,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: view_projection_buffer.as_entire_binding(),
        }],
        label: Some("Camera Bind Group"),
    });

    (layout, bind_group)
}

fn create_texture_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Texture Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

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
