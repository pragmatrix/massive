use std::{mem, result};

use log::info;
use massive_geometry::Matrix4;
use wgpu::{util::DeviceExt, StoreOp};

use crate::{
    pods::{self, TextureVertex},
    primitives::{Pipeline, Primitive},
    shape,
    texture::{self, Texture},
    tools::BindGroupLayoutBuilder,
};

pub struct Renderer<'window> {
    surface: wgpu::Surface<'window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,

    view_projection_buffer: wgpu::Buffer,
    view_projection_bind_group: wgpu::BindGroup,
    pub texture_sampler: wgpu::Sampler,
    pub texture_bind_group_layout: texture::BindGroupLayout,

    pipelines: Vec<(Pipeline, wgpu::RenderPipeline)>,

    quad_index_buffer: wgpu::Buffer,
}

impl<'window> Renderer<'window> {
    /// Creates a new renderer and reconfigures the surface according to the given configuration.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'window>,
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

        let shape_bind_group_layout = shape::BindGroupLayout::new(&device);

        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(Self::QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let pipelines = {
            let targets = [Some(wgpu::ColorTargetState {
                format: surface_config.format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })];

            create_pipelines(
                &device,
                &view_projection_bind_group_layout,
                &texture_bind_group_layout,
                &shape_bind_group_layout,
                &targets,
            )
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

    // TODO: Can't we handle SurfaceError::Lost here by just reconfiguring the surface and trying
    // again?
    pub fn render_and_present(
        &mut self,
        view_projection_matrix: &Matrix4,
        primitives: &[Primitive],
    ) -> result::Result<(), wgpu::SurfaceError> {
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.queue_view_projection_matrix(view_projection_matrix);

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
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                for pipeline in &self.pipelines {
                    let kind = pipeline.0;
                    let pipeline = &pipeline.1;
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(0, &self.view_projection_bind_group, &[]);
                    render_pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );

                    for primitive in primitives.iter().filter(|p| p.pipeline() == kind) {
                        match primitive {
                            Primitive::Texture(Texture {
                                bind_group,
                                vertex_buffer,
                                ..
                            }) => {
                                render_pass.set_bind_group(1, bind_group, &[]);
                                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                                render_pass.draw_indexed(
                                    0..Self::QUAD_INDICES.len() as u32,
                                    0,
                                    0..1,
                                );
                            }
                        }
                    }
                }
            }
            encoder.finish()
        };

        self.queue.submit([command_buffer]);
        surface_texture.present();
        Ok(())
    }

    const QUAD_INDICES: &'static [u16] = &[0, 1, 2, 0, 2, 3];

    fn queue_view_projection_matrix(&self, view_projection_matrix: &Matrix4) {
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

    // A Matrix that projects from normalized view coordinates -1.0 to 1.0 (3D, all axis, Z from 0.1
    // to 100) to 2D coordinates.

    // A Matrix that translates from the WGPU coordinate system to surface coordinates.
    pub fn surface_matrix(&self) -> Matrix4 {
        let (width, height) = self.surface_size();
        Matrix4::from_nonuniform_scale(width as f64 / 2.0, (height as f64 / 2.0) * -1.0, 1.0)
            * Matrix4::from_translation(cgmath::Vector3::new(1.0, -1.0, 0.0))
    }

    /// Resizes the surface, if necessary.
    /// Keeps the surface size at least 1x1.
    pub fn resize_surface(&mut self, new_size: (u32, u32)) {
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
    pub fn surface_size(&self) -> (u32, u32) {
        let config = &self.surface_config;
        (config.width, config.height)
    }

    pub fn reconfigure_surface(&mut self) {
        info!("Reconfiguring surface {:?}", self.surface_config);
        self.surface.configure(&self.device, &self.surface_config)
    }
}

fn create_pipelines(
    device: &wgpu::Device,
    view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
    shape_bind_group_layout: &wgpu::BindGroupLayout,
    targets: &[Option<wgpu::ColorTargetState>],
) -> Vec<(Pipeline, wgpu::RenderPipeline)> {
    let glyph_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Glyph Pipeline Layout"),
        bind_group_layouts: &[view_projection_bind_group_layout, texture_bind_group_layout],
        push_constant_ranges: &[],
    });

    let glyph_shader = &device.create_shader_module(wgpu::include_wgsl!("texture/glyph.wgsl"));

    let shape_shader = &device.create_shader_module(wgpu::include_wgsl!("shape/shape.wgsl"));

    let shape_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shape Pipeline Layout"),
        bind_group_layouts: &[view_projection_bind_group_layout, shape_bind_group_layout],
        push_constant_ranges: &[],
    });

    [
        (
            Pipeline::PlanarGlyph,
            create_pipeline(
                "Planar Glyph Pipeline",
                device,
                glyph_shader,
                "fs_planar",
                &glyph_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::SdfGlyph,
            create_pipeline(
                "SDF Glyph Pipeline",
                device,
                glyph_shader,
                "fs_sdf_glyph",
                &glyph_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::Circle,
            create_pipeline(
                "Circle Pipeline",
                device,
                shape_shader,
                "fs_sdf_circle",
                &shape_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::RoundedRect,
            create_pipeline(
                "Rounded Rect Pipeline",
                device,
                shape_shader,
                "fs_sdf_rounded_rect",
                &shape_pipeline_layout,
                targets,
            ),
        ),
    ]
    .into()
}

fn create_view_projection_bind_group(
    device: &wgpu::Device,
    view_projection_buffer: &wgpu::Buffer,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = BindGroupLayoutBuilder::vertex()
        .uniform()
        .build("Camera Bind Group Layout", device);

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
    fragment_shader_entry: &str,
    render_pipeline_layout: &wgpu::PipelineLayout,
    targets: &[Option<wgpu::ColorTargetState>],
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
