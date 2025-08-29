use std::{
    collections::HashMap,
    result,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::Result;
use cosmic_text::FontSystem;
use itertools::Itertools;
use log::{info, warn};
use massive_shapes::Shape;
use wgpu::{PresentMode, StoreOp, SurfaceTexture};

use crate::{
    Transaction, TransactionManager,
    pods::{AsBytes, ToPod},
    scene::{LocationMatrices, Scene},
    shape_renderer::ShapeRenderer,
    stats::MeasureSeries,
    text_layer::TextLayerRenderer,
    tools::QuadIndexBuffer,
};
use massive_geometry::{Color, Matrix4};
use massive_scene::{ChangedIds, Id, SceneChange, VisualRenderObj};

#[derive(Debug)]
pub struct Renderer {
    config: Config,
    surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub measure_series: MeasureSeries,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub background_color: Option<Color>,

    quads_index_buffer: QuadIndexBuffer,
    max_quads_in_use: usize,

    //
    // Scene and Cache updates
    //
    transaction_manager: TransactionManager,
    scene: Scene,

    /// The changed visuals since the previous draw call.
    changed_visuals: ChangedIds,

    visual_matrices: LocationMatrices,
    /// Per visual location and pipeline batches.
    visuals: HashMap<Id, RenderVisual>,

    //
    // Pipeline Renderer
    //
    text_renderer: TextLayerRenderer,
    shape_renderer: ShapeRenderer,
}

#[derive(Debug)]
pub struct RenderVisual {
    pub location_id: Id,
    pub batches: PipelineBatches,
}

/// Representing all batches in a visual.
#[derive(Debug)]
pub struct PipelineBatches {
    pub sdf: Option<RenderBatch>,
    pub color: Option<RenderBatch>,
    pub shapes: Option<RenderBatch>,
}

impl PipelineBatches {
    pub fn max_quads(&self) -> usize {
        [
            self.sdf.as_ref().map(|b| b.count).unwrap_or_default(),
            self.color.as_ref().map(|b| b.count).unwrap_or_default(),
            self.shapes.as_ref().map(|b| b.count).unwrap_or_default(),
        ]
        .into_iter()
        .max()
        .unwrap()
    }
}

#[derive(Debug)]
pub struct RenderBatch {
    pub fs_bind_group: Option<wgpu::BindGroup>,
    pub vertex_buffer: wgpu::Buffer,
    pub count: usize,
}

/// The context provided to `prepare()` middleware functions.
pub struct PreparationContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

pub struct RenderContext<'a> {
    pub pixel_matrix: &'a Matrix4,
    pub view_projection_matrix: Matrix4,
    pub pass: wgpu::RenderPass<'a>,
}

#[derive(Debug)]
pub struct Config {
    measure: bool,
}

const MEASURE: bool = true;

impl Renderer {
    /// Creates a new renderer and reconfigures the surface according to the given configuration.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        font_system: Arc<Mutex<FontSystem>>,
    ) -> Self {
        let format = surface_config.format;

        let text_layer_renderer = TextLayerRenderer::new(&device, font_system, format);
        let shape_renderer = crate::shape_renderer::ShapeRenderer::new::<
            crate::shape_renderer::Vertex,
        >(&device, format);

        // Currently unused (and also not anti-aliased). Need to create an sdf shape renderer.
        // let quads_renderer =
        //     QuadsRenderer::new(&device, format, &view_projection_bind_group_layout);

        let index_buffer = QuadIndexBuffer::new(&device);

        let mut renderer = Self {
            config: Config { measure: MEASURE },
            device,
            queue,
            measure_series: Default::default(),
            surface,
            surface_config,
            background_color: Some(Color::WHITE),
            quads_index_buffer: index_buffer,
            max_quads_in_use: 0,

            transaction_manager: Default::default(),
            scene: Default::default(),
            changed_visuals: Default::default(),
            visual_matrices: Default::default(),
            visuals: Default::default(),

            text_renderer: text_layer_renderer,
            shape_renderer,
        };

        renderer.reconfigure_surface();
        renderer
    }

    /// Apply changes to the renderer hierarchy.
    ///
    /// This can be called multiple times with new changes without losing significant performance compared to combining all changes first. I.e. no expensive value computation is done here.
    ///
    /// After all changes are pushed, call prepare().
    #[tracing::instrument(skip_all)]
    pub fn apply_changes(&mut self, changes: impl IntoIterator<Item = SceneChange>) -> Result<()> {
        let transaction = self.transaction_manager.new_transaction();

        for change in changes {
            self.scene.apply(&change, &transaction);
            if let SceneChange::Visual(visual_change) = change {
                self.changed_visuals.add(visual_change.id());
            }
        }
        Ok(())
    }

    /// Prepare everything we can before the final render, without changing GPU state.
    ///
    /// Currently this
    /// - produces all pipeline batches.
    /// - prepares the index buffer so that it can render all quads needed.
    /// - computes the final matrices for all the visuals.
    ///
    /// It's important that pipeline batches are updated before index buffer and final matrices,
    /// because they both depend on it. The index buffer needs the max quads used, and the final
    /// matrix computation needs a unique list of locations.
    ///
    /// Architecture: These dependencies should probably be encoded with the type system.
    ///
    /// The prepare step should run before get_current_texture(), because it would block, and wait
    /// for the next VSync. If we run prepare steps before, we can utilize CPU time more.
    pub fn prepare(&mut self) -> Result<()> {
        self.prepare_batches()?;

        self.prepare_index_buffer();

        {
            let transaction = self.transaction_manager.current_transaction();
            self.prepare_matrices(&transaction);
        }

        Ok(())
    }

    fn prepare_index_buffer(&mut self) {
        // Performance: Compute max_quads them incrementally, going through all batches might be
        // expensive.
        //
        // Compute only one max_quads value (which is optimal when we use one index buffer only).
        let max_quads = self
            .visuals
            .values()
            .map(|v| v.batches.max_quads())
            .max()
            .unwrap_or_default();

        self.quads_index_buffer
            .ensure_can_index_num_quads(&self.device, max_quads);

        self.max_quads_in_use = max_quads;
    }

    fn prepare_matrices(&mut self, transaction: &Transaction) {
        let location_ids = self.visuals.values().map(|v| v.location_id).unique();
        self.visual_matrices
            .compute_matrices(&self.scene, transaction, location_ids);
    }

    fn prepare_batches(&mut self) -> Result<()> {
        let visuals = self.scene.visuals();
        for id in self.changed_visuals.take_all() {
            if let Some(v) = &visuals[id] {
                Self::visual_updated(
                    id,
                    v,
                    &self.device,
                    &self.queue,
                    &mut self.text_renderer,
                    &self.shape_renderer,
                    &mut self.visuals,
                )?;
            } else {
                self.visuals.remove(&id);
            }
        }

        Ok(())
    }

    pub fn visual_updated(
        id: Id,
        visual: &VisualRenderObj,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &mut TextLayerRenderer,
        shape_renderer: &ShapeRenderer,
        render_visuals: &mut HashMap<Id, RenderVisual>,
    ) -> Result<()> {
        let runs = visual.shapes.iter().filter_map(|s| match s {
            Shape::GlyphRun(run) => Some(run),
            _ => None,
        });

        let batches = text_renderer.runs_to_batches(&PreparationContext { device, queue }, runs)?;

        let shape_batch = shape_renderer
            .batch_from_shapes(device, &visual.shapes)
            .map(|b| RenderBatch {
                fs_bind_group: None,
                vertex_buffer: b.vertex_buffer,
                count: b.quad_count,
            });

        render_visuals.insert(
            id,
            RenderVisual {
                location_id: visual.location,
                batches: PipelineBatches {
                    sdf: batches.sdf,
                    color: batches.color,
                    shapes: shape_batch,
                },
            },
        );
        Ok(())
    }

    /// We want this separate from [`Self::render_and_present`], because of the timing impliciation.
    /// In any VSync mode, this blocks until the current frame is presented.
    ///
    /// This is `&mut self`, because it might call into [`Self::reconfigure_surface`] when the
    /// surface is lost.
    pub fn get_current_texture(&mut self) -> result::Result<SurfaceTexture, wgpu::SurfaceError> {
        match self.surface.get_current_texture() {
            Ok(texture) => Ok(texture),
            Err(e) => {
                // Try to reconfigure and re-acquire once when the surface is lost.
                warn!("Surface error: {e:?}, retrying...");
                self.reconfigure_surface();
                self.surface.get_current_texture()
            }
        }
    }

    // TODO: Can't we handle SurfaceError::Lost here by just reconfiguring the surface and trying
    // again?
    #[tracing::instrument(skip_all)]
    pub fn render_and_present(
        &mut self,
        view_projection_matrix: &Matrix4,
        surface_texture: SurfaceTexture,
    ) {
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let pixel_matrix = self.pixel_matrix();

        let render_start_time = Instant::now();

        let command_buffer = {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

            {
                let load_op = if let Some(color) = self.background_color {
                    let (r, g, b, a) = color.into();
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as _,
                        g: g as _,
                        b: b as _,
                        a: a as _,
                    })
                } else {
                    wgpu::LoadOp::Load
                };

                let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: load_op,
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                // DI: There is a lot of view_projection stuff going on.
                let render_context = &mut RenderContext {
                    pixel_matrix: &pixel_matrix,
                    pass: render_pass,
                    view_projection_matrix: *view_projection_matrix,
                };

                // Set the shared index buffer for all quad renderers.
                if self.max_quads_in_use > 0 {
                    self.quads_index_buffer
                        .set(&mut render_context.pass, self.max_quads_in_use);
                }

                self.render_pipeline_batches(
                    self.text_renderer.sdf_pipeline(),
                    |b| b.sdf.as_ref(),
                    render_context,
                );

                self.render_pipeline_batches(
                    self.text_renderer.color_pipeline(),
                    |b| b.color.as_ref(),
                    render_context,
                );

                self.render_pipeline_batches(
                    self.shape_renderer.pipeline(),
                    |b| b.shapes.as_ref(),
                    render_context,
                );
            }
            encoder.finish()
        };

        let submit_index = self.queue.submit([command_buffer]);

        if self.config.measure {
            // Robustness: This should be done in another thread to prevent us from blocking or delaying present().
            self.device
                .poll(wgpu::PollType::WaitForSubmissionIndex(submit_index))
                .unwrap();
            let duration_passed = Instant::now().duration_since(render_start_time);
            self.measure_series.add_sample(duration_passed);
        }

        surface_texture.present();
    }

    /// Pick up one specific pipeline batch from every visual and render it.
    pub fn render_pipeline_batches(
        &self,
        pipeline: &wgpu::RenderPipeline,
        select_batch: impl Fn(&PipelineBatches) -> Option<&RenderBatch>,
        context: &mut RenderContext,
    ) {
        let matrices = &self.visual_matrices;
        context.pass.set_pipeline(pipeline);

        for visual in self.visuals.values() {
            if let Some(batch) = select_batch(&visual.batches) {
                // Architecture: We may go multiple times over the same visual and compute the
                //   final, because it renders to different pipelines. Perhaps we need a derived /
                //   lazy table here.
                let model_matrix = context.pixel_matrix * matrices.get(visual.location_id);
                let matrix = context.view_projection_matrix * model_matrix;

                let pass = &mut context.pass;

                pass.set_push_constants(wgpu::ShaderStages::VERTEX, 0, matrix.to_pod().as_bytes());
                // Architecture: This test needs only done once per pipeline.
                if let Some(bg) = &batch.fs_bind_group {
                    pass.set_bind_group(0, bg, &[]);
                }
                pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));

                pass.draw_indexed(
                    0..(batch.count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
                    0,
                    0..1,
                )
            }
        }
    }

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    pub fn pixel_matrix(&self) -> Matrix4 {
        let (_, surface_height) = self.surface_size();
        Matrix4::from_nonuniform_scale(1.0, -1.0, 1.0)
            * Matrix4::from_scale(1.0 / surface_height as f64 * 2.0)
    }

    /// A Matrix that projects from normalized view coordinates -1.0 to 1.0 (3D, all axis, Z from 0.1
    /// to 100) to 2D coordinates.
    ///
    /// A Matrix that translates from the WGPU coordinate system to surface coordinates.
    pub fn surface_matrix(&self) -> Matrix4 {
        let (width, height) = self.surface_size();
        Matrix4::from_nonuniform_scale(width as f64 / 2.0, -(height as f64 / 2.0), 1.0)
            * Matrix4::from_translation(cgmath::Vector3::new(1.0, -1.0, 0.0))
    }

    /// Resizes the surface, if necessary.
    ///
    /// Keeps the minimum surface size at at least 1x1.
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
    ///
    /// It may not exactly match the window's size, for example if the window's size is 0,0, the
    /// surface's size will be 1x1.
    pub fn surface_size(&self) -> (u32, u32) {
        let config = &self.surface_config;
        (config.width, config.height)
    }

    pub fn present_mode(&self) -> PresentMode {
        self.surface_config.present_mode
    }

    /// Sets the presentation mode and - if changed - reconfigures the surface.
    pub fn set_present_mode(&mut self, present_mode: PresentMode) {
        if present_mode == self.surface_config.present_mode {
            return;
        }
        self.surface_config.present_mode = present_mode;
        self.reconfigure_surface();
    }

    pub fn reconfigure_surface(&mut self) {
        info!("Reconfiguring surface {:?}", self.surface_config);
        self.surface.configure(&self.device, &self.surface_config);
    }
}
