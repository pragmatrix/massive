use std::{
    collections::HashSet,
    result,
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::pods::{AsBytes, ToPod};
use anyhow::Result;
use cosmic_text::FontSystem;
use log::{info, warn};
use massive_shapes::Shape;
use wgpu::{PresentMode, StoreOp, SurfaceTexture};

use crate::{
    TransactionManager,
    scene::{IdTable, LocationMatrices, Scene},
    stats::MeasureSeries,
    text_layer::TextLayerRenderer,
};
use massive_geometry::{Color, Matrix4};
use massive_scene::{Change, Id, SceneChange, VisualRenderObj};

#[derive(Debug)]
pub struct Renderer {
    config: Config,
    surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub measure_series: MeasureSeries,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub background_color: Option<Color>,

    transaction_manager: TransactionManager,
    scene: Scene,
    visual_matrices: LocationMatrices,
    /// Visual Id -> batch table.
    visuals: IdTable<Option<RenderVisual>>,

    text_layer_renderer: TextLayerRenderer,
    shape_renderer: crate::shape_renderer::ShapeRenderer,
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
    pub shapes: Option<ShapeRenderBatch>,
}

impl PipelineBatches {
    pub fn max_quads(&self) -> usize {
        [
            self.sdf.as_ref().map(|b| b.count).unwrap_or_default(),
            self.color.as_ref().map(|b| b.count).unwrap_or_default(),
        ]
        .into_iter()
        .max()
        .unwrap()
    }
}

#[derive(Debug)]
pub struct RenderBatch {
    /// The bind group contains texture reference(s) and the sampler configuration.
    pub fs_bind_group: wgpu::BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub count: usize,
}

#[derive(Debug)]
pub struct ShapeRenderBatch {
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

        let mut renderer = Self {
            config: Config { measure: MEASURE },
            device,
            queue,
            surface,
            surface_config,
            measure_series: MeasureSeries::default(),
            transaction_manager: TransactionManager::default(),
            scene: Scene::default(),
            visual_matrices: LocationMatrices::default(),
            text_layer_renderer,
            shape_renderer,
            background_color: Some(Color::WHITE),
            visuals: IdTable::default(),
            // quads_renderer,
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

        // Architecture:
        //
        // Because there are no interdependencies beetween the text layer renderer and the scene, we
        // could apply them in batches (and even paralellize)

        // Optimization:
        //
        // I don't think that the scene needs to store Visuals anymore. All that is needed is
        // extracted in the text_layer_renderer.
        for change in changes {
            // Optimization: Parallelize?
            self.scene.apply(&change, &transaction);
            self.apply(&change)?;
        }
        Ok(())
    }

    // Architecture: Optimization:
    //
    // This immediately creates QuadBatches, meaning that if we apply a Create / Delete combination
    // they would be destroyed before rendered. I think that we should create the QuadBatches later
    // based on a actual usage (and even later visibility) analysis?
    pub fn apply(&mut self, change: &SceneChange) -> Result<()> {
        if let SceneChange::Visual(visual_change) = change {
            match visual_change {
                Change::Create(id, visual) | Change::Update(id, visual) => {
                    self.insert(*id, visual)?;
                }
                Change::Delete(id) => {
                    self.delete(*id);
                }
            }
        }
        Ok(())
    }

    pub fn insert(&mut self, id: Id, visual: &VisualRenderObj) -> Result<()> {
        let runs = visual.shapes.iter().filter_map(|s| match s {
            Shape::GlyphRun(run) => Some(run),
            _ => None,
        });

        // Collect shape instances for non-text shapes.
        #[derive(Copy, Clone)]
        struct ShapeInstanceData {
            vertices: [crate::shape_renderer::Vertex; 4],
        }

        impl crate::shape_renderer::ShapeInstance for ShapeInstanceData {
            type Vertex = crate::shape_renderer::Vertex;
            fn to_vertices(&self) -> [Self::Vertex; 4] {
                self.vertices
            }
        }

        use massive_geometry::Point;
        let mut shape_instances: Vec<ShapeInstanceData> = Vec::new();
        for shape in visual.shapes.iter() {
            match shape {
                Shape::GlyphRun(_) => {}
                Shape::Rect(r) => {
                    let w = (r.rect.right - r.rect.left) as f32;
                    let h = (r.rect.bottom - r.rect.top) as f32;
                    let selector = 0u32; // filled rect
                    let size = (w, h);
                    let data = (0.0, 0.0);
                    let color = r.color;
                    let lt: Point = (r.rect.left, r.rect.top).into();
                    let rb: Point = (r.rect.right, r.rect.bottom).into();
                    let lb: Point = (r.rect.left, r.rect.bottom).into();
                    let rt: Point = (r.rect.right, r.rect.top).into();
                    // Add 1px AA border around geometry in model coordinates.
                    let b = 1.0f32;
                    let verts = [
                        crate::shape_renderer::Vertex::new(
                            ((lt.x as f32) - b, (lt.y as f32) - b, 0.0),
                            (-b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((lb.x as f32) - b, (lb.y as f32) + b, 0.0),
                            (-b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rb.x as f32) + b, (rb.y as f32) + b, 0.0),
                            (w + b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rt.x as f32) + b, (rt.y as f32) - b, 0.0),
                            (w + b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ];
                    shape_instances.push(ShapeInstanceData { vertices: verts });
                }
                Shape::RoundRect(r) => {
                    let w = (r.rect.right - r.rect.left) as f32;
                    let h = (r.rect.bottom - r.rect.top) as f32;
                    let selector = 1u32; // rounded rect
                    let size = (w, h);
                    let data = (r.corner_radius, 0.0);
                    let color = r.color;
                    let lt: Point = (r.rect.left, r.rect.top).into();
                    let rb: Point = (r.rect.right, r.rect.bottom).into();
                    let lb: Point = (r.rect.left, r.rect.bottom).into();
                    let rt: Point = (r.rect.right, r.rect.top).into();
                    let b = 1.0f32;
                    let verts = [
                        crate::shape_renderer::Vertex::new(
                            ((lt.x as f32) - b, (lt.y as f32) - b, 0.0),
                            (-b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((lb.x as f32) - b, (lb.y as f32) + b, 0.0),
                            (-b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rb.x as f32) + b, (rb.y as f32) + b, 0.0),
                            (w + b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rt.x as f32) + b, (rt.y as f32) - b, 0.0),
                            (w + b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ];
                    shape_instances.push(ShapeInstanceData { vertices: verts });
                }
                Shape::Circle(c) => {
                    let w = (c.rect.right - c.rect.left) as f32;
                    let h = (c.rect.bottom - c.rect.top) as f32;
                    let selector = 2u32; // circle
                    let size = (w, h);
                    let data = (0.0, 0.0);
                    let color = c.color;
                    let lt: Point = (c.rect.left, c.rect.top).into();
                    let rb: Point = (c.rect.right, c.rect.bottom).into();
                    let lb: Point = (c.rect.left, c.rect.bottom).into();
                    let rt: Point = (c.rect.right, c.rect.top).into();
                    let b = 1.0f32;
                    let verts = [
                        crate::shape_renderer::Vertex::new(
                            ((lt.x as f32) - b, (lt.y as f32) - b, 0.0),
                            (-b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((lb.x as f32) - b, (lb.y as f32) + b, 0.0),
                            (-b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rb.x as f32) + b, (rb.y as f32) + b, 0.0),
                            (w + b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rt.x as f32) + b, (rt.y as f32) - b, 0.0),
                            (w + b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ];
                    shape_instances.push(ShapeInstanceData { vertices: verts });
                }
                Shape::StrokeRect(s) => {
                    let w = (s.rect.right - s.rect.left) as f32;
                    let h = (s.rect.bottom - s.rect.top) as f32;
                    let selector = 3u32; // stroke rect
                    let size = (w, h);
                    let data = (s.stroke.width as f32, s.stroke.height as f32);
                    let color = s.color;
                    let lt: Point = (s.rect.left, s.rect.top).into();
                    let rb: Point = (s.rect.right, s.rect.bottom).into();
                    let lb: Point = (s.rect.left, s.rect.bottom).into();
                    let rt: Point = (s.rect.right, s.rect.top).into();
                    let b = 1.0f32;
                    let verts = [
                        crate::shape_renderer::Vertex::new(
                            ((lt.x as f32) - b, (lt.y as f32) - b, 0.0),
                            (-b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((lb.x as f32) - b, (lb.y as f32) + b, 0.0),
                            (-b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rb.x as f32) + b, (rb.y as f32) + b, 0.0),
                            (w + b, h + b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        crate::shape_renderer::Vertex::new(
                            ((rt.x as f32) + b, (rt.y as f32) - b, 0.0),
                            (w + b, -b),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ];
                    shape_instances.push(ShapeInstanceData { vertices: verts });
                }
            }
        }

        let context = &mut PreparationContext {
            device: &self.device,
            queue: &self.queue,
        };

        let batches = self.text_layer_renderer.runs_to_batches(context, runs)?;
        // Shape batch
        let shape_batch = self
            .shape_renderer
            .batch(&self.device, &shape_instances)
            .map(|b| ShapeRenderBatch {
                vertex_buffer: b.vertex_buffer,
                count: b.quad_count,
            });
        self.visuals.insert(
            id,
            Some(RenderVisual {
                location_id: visual.location,
                batches: PipelineBatches {
                    sdf: batches.sdf,
                    color: batches.color,
                    shapes: shape_batch,
                },
            }),
        );
        Ok(())
    }

    pub fn delete(&mut self, id: Id) {
        self.visuals[id] = None;
    }

    pub fn all_locations(&self) -> impl Iterator<Item = Id> + use<> {
        let mut locations = HashSet::new();
        for visual in self.visuals.iter_some() {
            locations.insert(visual.location_id);
        }
        locations.into_iter()
    }

    /// Prepare everything we can before the final render, without changing GPU state.
    ///
    /// Currently this
    /// - This computes all final matrices.
    ///
    /// The prepare step should run before get_current_texture(), because it would block, and wait
    /// for the next VSync. If we run prepare steps before, we can utilize CPU time more.
    pub fn prepare(&mut self) {
        let context = &mut PreparationContext {
            device: &self.device,
            queue: &self.queue,
        };

        self.text_layer_renderer.prepare(&self.visuals, context);

        let transaction = self.transaction_manager.current_transaction();

        // Compute visuals locations
        {
            let all_locations = self.all_locations();
            self.visual_matrices.compute_matrices(
                &self.scene,
                &transaction,
                all_locations.into_iter(),
            );
        }
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
                let mut render_context = RenderContext {
                    pixel_matrix: &pixel_matrix,
                    pass: render_pass,
                    view_projection_matrix: *view_projection_matrix,
                };

                self.text_layer_renderer.render(
                    &self.visual_matrices,
                    &self.visuals,
                    &mut render_context,
                );

                // Shape rendering
                let mut max_shape_quads = 0usize;
                for visual in self.visuals.iter_some() {
                    if let Some(ref shape_batch) = visual.batches.shapes {
                        max_shape_quads = max_shape_quads.max(shape_batch.count);
                    }
                }
                if max_shape_quads > 0 {
                    self.shape_renderer
                        .ensure_index_capacity(&self.device, max_shape_quads);
                    render_context
                        .pass
                        .set_pipeline(self.shape_renderer.pipeline());
                    self.shape_renderer
                        .set_index_buffer(&mut render_context.pass, max_shape_quads);
                    for visual in self.visuals.iter_some() {
                        if let Some(ref shape_batch) = visual.batches.shapes {
                            let model_matrix = render_context.pixel_matrix
                                * self.visual_matrices.get(visual.location_id);
                            let vm = render_context.view_projection_matrix * model_matrix;
                            render_context.pass.set_push_constants(
                                wgpu::ShaderStages::VERTEX,
                                0,
                                vm.to_pod().as_bytes(),
                            );
                            render_context
                                .pass
                                .set_vertex_buffer(0, shape_batch.vertex_buffer.slice(..));
                            render_context.pass.draw_indexed(
                                0..(shape_batch.count
                                    * crate::tools::QuadIndexBuffer::INDICES_PER_QUAD)
                                    as u32,
                                0,
                                0..1,
                            );
                        }
                    }
                }
                // self.quads_renderer.render(&mut render_context);

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
