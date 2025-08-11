use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use swash::{Weight, scale::ScaleContext};
use text::SwashContent;
use wgpu::Device;

use crate::{
    glyph::{
        GlyphRasterizationParam, RasterizedGlyphKey, SwashRasterizationParam, glyph_atlas,
        glyph_rasterization::rasterize_glyph_with_padding,
    },
    pods::{AsBytes, ToPod},
    renderer::{PreparationContext, RenderContext},
    scene::{IdTable, LocationMatrices},
    text_layer::atlas_renderer::{self, AtlasRenderer, color_atlas, sdf_atlas},
    tools::QuadIndexBuffer,
};
use massive_scene::{Change, Id, Matrix, SceneChange, Shape, VisualRenderObj};
use massive_shapes::{GlyphRun, RunGlyph, TextWeight};

pub struct TextLayerRenderer {
    // Optimization: This is used for get_font() only, which needs &mut. In the long run, completely
    // put the character renderer off-thread and run the rasterizers completely parallel (tokio is
    // probably fine, too). This is needed as soon we need asynchronous optimization of rendered
    // resolutions to match the pixel density.
    // Architecture: We should wrap this in some kind of FontEnvironment, or RasterizerEnvironment?
    font_system: Arc<Mutex<FontSystem>>,
    // Font cache and scratch buffers for the rasterizer.
    //
    // TODO: May make the Rasterizer a thing and put it in there alongside with its functions. This
    // would allow further optimizations I guess (e.g. an own scratch buffer, etc.).
    scale_context: ScaleContext,
    empty_glyphs: HashSet<RasterizedGlyphKey>,

    index_buffer: QuadIndexBuffer,

    sdf_renderer: AtlasRenderer,
    color_renderer: AtlasRenderer,

    // Architecture:
    //
    // Visuals should be stored one layer above. After all, they contain all shapes,
    // Quads for example?
    /// Visual Id -> batch table.
    visuals: IdTable<Option<Visual>>,
}

struct Visual {
    location_id: Id,
    batches: VisualBatches,
}

/// Representing all batches in a visual.
struct VisualBatches {
    sdf: Option<QuadBatch>,
    color: Option<QuadBatch>,
}

pub struct QuadBatch {
    /// Contains texture reference(s) and the sampler configuration.
    pub fs_bind_group: wgpu::BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub quad_count: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum AtlasKind {
    Sdf,
    Color,
}

impl TextLayerRenderer {
    pub fn new(
        device: &Device,
        font_system: Arc<Mutex<FontSystem>>,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let mut renderer = Self {
            scale_context: ScaleContext::default(),
            font_system,
            empty_glyphs: HashSet::new(),
            index_buffer: QuadIndexBuffer::new(device),
            // Instead of specifying all these consts _and_ the vertex type, a trait based spec type
            // would probably be better.
            sdf_renderer: AtlasRenderer::new::<atlas_renderer::sdf_atlas::Instance>(
                device,
                wgpu::TextureFormat::R8Unorm,
                wgpu::include_wgsl!("shader/sdf_atlas.wgsl"),
                target_format,
            ),
            color_renderer: AtlasRenderer::new::<atlas_renderer::color_atlas::Instance>(
                device,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::include_wgsl!("shader/color_atlas.wgsl"),
                target_format,
            ),
            visuals: IdTable::default(),
        };

        // Since we use instance drendering, the index buffer needs to hold only one quad.
        renderer.index_buffer.ensure_can_index_num_quads(device, 1);
        renderer
    }

    // Architecture: Optimization:
    //
    // This immediately creates QuadBatches, meaning that if we apply a Create / Delete combination
    // they would be destroyed before rendered. I think that we should create the QuadBatches later
    // based on a actual usage (and even later visibility) analysis?
    pub fn apply(&mut self, change: &SceneChange, context: &mut PreparationContext) -> Result<()> {
        if let SceneChange::Visual(visual_change) = change {
            match visual_change {
                Change::Create(id, visual) | Change::Update(id, visual) => {
                    self.insert(*id, visual, context)?;
                }
                Change::Delete(id) => {
                    self.delete(*id);
                }
            }
        }
        Ok(())
    }

    pub fn insert(
        &mut self,
        id: Id,
        visual: &VisualRenderObj,
        context: &mut PreparationContext,
    ) -> Result<()> {
        let runs = visual.shapes.iter().filter_map(|s| match s {
            Shape::GlyphRun(run) => Some(run),
            Shape::Quads(_) => None,
        });

        let batches = self.runs_to_batches(context, runs)?;
        self.visuals.insert(
            id,
            Some(Visual {
                location_id: visual.location,
                batches,
            }),
        );
        Ok(())
    }

    pub fn delete(&mut self, id: Id) {
        self.visuals[id] = None;
    }

    pub fn all_locations(&self) -> impl Iterator<Item = Id> {
        let mut locations = HashSet::new();
        for visual in self.visuals.iter_some() {
            locations.insert(visual.location_id);
        }
        locations.into_iter()
    }

    pub fn prepare(&mut self, _context: &mut PreparationContext) {}

    pub fn render(&self, matrices: &LocationMatrices, context: &mut RenderContext) {
        // Set the shared index buffer for all quad renderers.
        self.index_buffer.set(&mut context.pass, 1);

        {
            context.pass.set_pipeline(self.sdf_renderer.pipeline());

            for visual in self.visuals.iter_some() {
                if let Some(ref sdf_batch) = visual.batches.sdf {
                    let model_matrix = context.pixel_matrix * matrices.get(visual.location_id);
                    Self::render_batch(context, &model_matrix, sdf_batch);
                }
            }
        }

        {
            context.pass.set_pipeline(self.color_renderer.pipeline());

            for visual in self.visuals.iter_some() {
                if let Some(ref color_batch) = visual.batches.color {
                    let model_matrix = context.pixel_matrix * matrices.get(visual.location_id);
                    Self::render_batch(context, &model_matrix, color_batch);
                }
            }
        }
    }

    pub fn render_batch(context: &mut RenderContext, model_matrix: &Matrix, batch: &QuadBatch) {
        let text_layer_matrix = context.view_projection_matrix * model_matrix;

        let pass = &mut context.pass;

        pass.set_push_constants(
            wgpu::ShaderStages::VERTEX,
            0,
            text_layer_matrix.to_pod().as_bytes(),
        );
        pass.set_bind_group(0, &batch.fs_bind_group, &[]);
        pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));

        // Draw instanced quads: 6 indices for the unit quad, one instance per glyph.
        pass.draw_indexed(
            0..(QuadIndexBuffer::INDICES_PER_QUAD as u32),
            0,
            0..(batch.quad_count as u32),
        )
    }

    /// Prepare a number of glyph runs and produce a TextLayer.
    ///
    /// All of the runs use the same model matrix.
    fn runs_to_batches<'a>(
        &mut self,
        context: &mut PreparationContext,
        runs: impl Iterator<Item = &'a GlyphRun>,
    ) -> Result<VisualBatches> {
        // Performance: Compute a conservative capacity?
        // Step 1: Build instance data directly.
        let mut sdf_instances: Vec<sdf_atlas::Instance> = Vec::new();
        let mut color_instances: Vec<color_atlas::Instance> = Vec::new();

        // Architecture: We should really not lock this for a longer period of time. After initial
        // renderers, it's usually not used anymore, because it just invokes get_font() on
        // non-existing glyphs.0
        let font_system = self.font_system.clone();
        let mut font_system = font_system.lock().unwrap();

        for run in runs {
            let translation = run.translation;
            for glyph in &run.glyphs {
                let Some((rect, placement, kind)) = self.rasterized_glyph_atlas_rect(
                    context,
                    &mut font_system,
                    run.text_weight,
                    glyph,
                )?
                else {
                    continue; // Glyph is empty: Not rendered.
                };

                // Compute left-top/right-bottom in pixel space once.
                let (lt, rb) = run.place_glyph(glyph, &placement);
                let left = lt.x as f32 + translation.x as f32;
                let top = lt.y as f32 + translation.y as f32;
                let right = rb.x as f32 + translation.x as f32;
                let bottom = rb.y as f32 + translation.y as f32;
                let depth = translation.z as f32;

                let pos_lt = [left, top];
                let pos_rb = [right, bottom];

                // Atlas rect in pixel space; normalization is done in shader.
                let uv_lt = [rect.min.x as f32, rect.min.y as f32];
                let uv_rb = [rect.max.x as f32, rect.max.y as f32];

                match kind {
                    AtlasKind::Sdf => {
                        sdf_instances.push(sdf_atlas::Instance {
                            pos_lt,
                            pos_rb,
                            uv_lt,
                            uv_rb,
                            color: run.text_color.into(),
                            depth,
                        });
                    }
                    AtlasKind::Color => {
                        color_instances.push(color_atlas::Instance {
                            pos_lt,
                            pos_rb,
                            uv_lt,
                            uv_rb,
                            depth,
                        });
                    }
                }
            }
        }

        let sdf_batch = self.sdf_renderer.batch_instances(context, &sdf_instances);
        let color_batch = self
            .color_renderer
            .batch_instances(context, &color_instances);

        Ok(VisualBatches {
            sdf: sdf_batch,
            color: color_batch,
        })
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &mut PreparationContext,
        font_system: &mut FontSystem,
        weight: TextWeight,
        glyph: &RunGlyph,
    ) -> Result<Option<(glyph_atlas::Rectangle, text::Placement, AtlasKind)>> {
        let glyph_key = RasterizedGlyphKey {
            text: glyph.key,
            param: GlyphRasterizationParam {
                prefer_sdf: true,
                swash: SwashRasterizationParam {
                    hinted: true,
                    weight: Weight(weight.0),
                },
            },
        };

        if let Some((rect, placement)) = self.sdf_renderer.atlas.get(&glyph_key) {
            return Ok(Some((rect, placement, AtlasKind::Sdf)));
        }

        if let Some((rect, placement)) = self.color_renderer.atlas.get(&glyph_key) {
            return Ok(Some((rect, placement, AtlasKind::Color)));
        }

        // Atlas / cache miss, empty cached glyph?.
        if self.empty_glyphs.contains(&glyph_key) {
            return Ok(None);
        }

        // Not yet in an atlas and not empty. Now rasterize.
        let Some(image) =
            rasterize_glyph_with_padding(font_system, &mut self.scale_context, &glyph_key)
        else {
            self.empty_glyphs.insert(glyph_key);
            return Ok(None);
        };

        let image_placement = image.placement;

        match image.content {
            SwashContent::Mask => {
                let rect_in_atlas = self.sdf_renderer.atlas.store(
                    context.device,
                    context.queue,
                    &glyph_key,
                    image,
                )?;

                Ok(Some((rect_in_atlas, image_placement, AtlasKind::Sdf)))
            }
            SwashContent::Color => {
                let rect_in_atlas = self.color_renderer.atlas.store(
                    context.device,
                    context.queue,
                    &glyph_key,
                    image,
                )?;

                Ok(Some((rect_in_atlas, image_placement, AtlasKind::Color)))
            }
            SwashContent::SubpixelMask => panic!("Unsupported Subpixel Mask"),
        }
    }

    // glyph_vertices removed in instanced rendering path
}
