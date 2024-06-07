use std::{
    mem::{self},
    result,
};

use anyhow::Result;
use log::info;
use massive_geometry::Matrix4;
use massive_scene::SceneChange;
use wgpu::StoreOp;

use crate::{
    pipelines, pods, quads::QuadsRenderer, scene::Scene, text, text_layer::TextLayerRenderer,
    texture,
};

pub struct Renderer<'window> {
    surface: wgpu::Surface<'window>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,

    scene: Scene,

    // DI: Type this.
    view_projection_buffer: wgpu::Buffer,
    // DI: Type this.
    view_projection_bind_group: wgpu::BindGroup,

    // TODO: this doesn't belong here and is used only for specific pipelines. We need some
    // per-pipeline information types.
    pub texture_bind_group_layout: texture::BindGroupLayout,

    text_layer_renderer: TextLayerRenderer,
    quads_renderer: QuadsRenderer,
}

/// The context provided to `prepare()` middleware functions.
pub struct PreparationContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub font_system: &'a mut text::FontSystem,
}

pub struct RenderContext<'a, 'rpass> {
    queue: &'a wgpu::Queue,
    view_projection_buffer: &'a wgpu::Buffer,
    pub view_projection_matrix: Matrix4,
    pub view_projection_bind_group: &'rpass wgpu::BindGroup,
    pub pass: &'a mut wgpu::RenderPass<'rpass>,
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
            pipelines::create_view_projection_bind_group(&device, &view_projection_buffer);

        let texture_bind_group_layout = texture::BindGroupLayout::new(&device);

        let format = surface_config.format;

        let text_layer_renderer =
            TextLayerRenderer::new(&device, format, &view_projection_bind_group_layout);

        let quads_renderer =
            QuadsRenderer::new(&device, format, &view_projection_bind_group_layout);

        let mut renderer = Self {
            device,
            queue,
            surface,
            surface_config,
            scene: Scene::default(),
            view_projection_buffer,
            view_projection_bind_group,
            texture_bind_group_layout,
            text_layer_renderer,
            quads_renderer,
        };

        renderer.reconfigure_surface();
        renderer
    }

    #[tracing::instrument(skip_all)]
    pub fn apply_changes(
        &mut self,
        font_system: &mut text::FontSystem,
        changes: impl IntoIterator<Item = SceneChange>,
    ) -> Result<()> {
        for change in changes {
            self.scene.apply(change);
        }

        let mut context = PreparationContext {
            device: &self.device,
            queue: &self.queue,
            font_system,
        };

        // OO: Lot's of allocations here.
        let grouped_shapes: Vec<_> = self.scene.grouped_shapes().collect();

        // OO: Lot's of allocations here.
        let grouped_by_matrix: Vec<_> = grouped_shapes
            .iter()
            .map(|(m, v)| (*m, v.as_slice()))
            .collect();

        // OO: parallelize?
        self.text_layer_renderer
            .prepare(&mut context, &grouped_by_matrix)?;
        self.quads_renderer
            .prepare(&mut context, &grouped_by_matrix)?;
        Ok(())
    }

    // TODO: Can't we handle SurfaceError::Lost here by just reconfiguring the surface and trying
    // again?
    #[tracing::instrument(skip_all)]
    pub fn render_and_present(
        &mut self,
        view_projection_matrix: &Matrix4,
    ) -> result::Result<(), wgpu::SurfaceError> {
        let surface_texture = self.surface.get_current_texture()?;
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // OO: This should not be needed anymore, because every renderer is now responsible for
        // setting up the view projection.
        Self::queue_view_projection_matrix(
            &self.queue,
            &self.view_projection_buffer,
            view_projection_matrix,
        );

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

                // DI: There is a lot of view_projection stuff going on.
                let mut render_context = RenderContext {
                    queue: &self.queue,
                    view_projection_buffer: &self.view_projection_buffer,
                    pass: &mut render_pass,
                    view_projection_matrix: *view_projection_matrix,
                    view_projection_bind_group: &self.view_projection_bind_group,
                };

                self.text_layer_renderer.render(&mut render_context);
                self.quads_renderer.render(&mut render_context);

                // for pipeline in &self.pipelines {
                //     let kind = pipeline.0;
                //     let pipeline = &pipeline.1;
                //     render_pass.set_pipeline(pipeline);
                //     render_pass.set_bind_group(0, &self.view_projection_bind_group, &[]);
                //     render_pass
                //         .set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                //     for primitive in primitives.iter().filter(|p| p.pipeline() == kind) {
                //         match primitive {
                //             Primitive::Texture(Texture {
                //                 bind_group,
                //                 vertex_buffer,
                //                 ..
                //             }) => {
                //                 render_pass.set_bind_group(1, bind_group, &[]);
                //                 render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                //                 render_pass.draw_indexed(
                //                     0..QuadIndexBuffer::QUAD_INDICES_COUNT as u32,
                //                     0,
                //                     0..1,
                //                 );
                //             }
                //         }
                //     }
                // }
            }
            encoder.finish()
        };

        self.queue.submit([command_buffer]);
        surface_texture.present();
        Ok(())
    }

    fn queue_view_projection_matrix(
        queue: &wgpu::Queue,
        view_projection_buffer: &wgpu::Buffer,
        view_projection_matrix: &Matrix4,
    ) {
        let view_projection_uniform = {
            let m: cgmath::Matrix4<f32> = view_projection_matrix
                .cast()
                .expect("matrix casting to f32 failed");
            pods::Matrix4(m.into())
        };

        queue.write_buffer(
            view_projection_buffer,
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

impl RenderContext<'_, '_> {
    pub fn queue_view_projection_matrix(&self, matrix: &Matrix4) {
        Renderer::queue_view_projection_matrix(self.queue, self.view_projection_buffer, matrix);
    }
}
