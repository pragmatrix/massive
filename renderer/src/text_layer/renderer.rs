use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use massive_geometry::{Point, Point3};
use massive_scene::{Change, Id, Matrix, SceneChange, Shape, VisualRenderObj};
use massive_shapes::{GlyphRun, RunGlyph, TextWeight};
use swash::{Weight, scale::ScaleContext};
use text::SwashContent;
use wgpu::Device;

use crate::{
    glyph::{
        GlyphRasterizationParam, RasterizedGlyphKey, SwashRasterizationParam, glyph_atlas,
        glyph_rasterization::rasterize_glyph_with_padding,
    },
    pods::{AsBytes, TextureColorVertex, TextureVertex, ToPod},
    renderer::{PreparationContext, RenderContext},
    scene::{IdTable, LocationMatrices},
    text_layer::atlas_renderer::{AtlasRenderer, color_atlas, sdf_atlas},
    tools::QuadIndexBuffer,
};

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

    /// The maximum quads currently in use. This may be more than the index buffer can hold.
    max_quads_in_use: usize,
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

impl VisualBatches {
    fn max_quads(&self) -> usize {
        [
            self.sdf.as_ref().map(|b| b.quad_count).unwrap_or_default(),
            self.color
                .as_ref()
                .map(|b| b.quad_count)
                .unwrap_or_default(),
        ]
        .into_iter()
        .max()
        .unwrap()
    }
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
        Self {
            scale_context: ScaleContext::default(),
            font_system,
            empty_glyphs: HashSet::new(),
            index_buffer: QuadIndexBuffer::new(device),
            // Instead of specifying all these consts _and_ the vertex type, a trait based spec type
            // would probably be better.
            sdf_renderer: AtlasRenderer::new::<TextureColorVertex>(
                device,
                wgpu::TextureFormat::R8Unorm,
                wgpu::include_wgsl!("shader/sdf_atlas.wgsl"),
                target_format,
            ),
            color_renderer: AtlasRenderer::new::<TextureVertex>(
                device,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::include_wgsl!("shader/color_atlas.wgsl"),
                target_format,
            ),
            visuals: IdTable::default(),
            max_quads_in_use: 0,
        }
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

    pub fn prepare(&mut self, context: &mut PreparationContext) {
        // Optimization: Visuals are iterated 4 times per render (see all_locations(), which could
        // also compute max_quads).

        // Compute only one max_quads value (which is optimal when we use one index buffer only).
        let max_quads = self
            .visuals
            .iter_some()
            .map(|v| v.batches.max_quads())
            .max()
            .unwrap_or_default();

        self.index_buffer
            .ensure_can_index_num_quads(context.device, max_quads);

        self.max_quads_in_use = max_quads;
    }

    pub fn render(&self, matrices: &LocationMatrices, context: &mut RenderContext) {
        if self.max_quads_in_use == 0 {
            return;
        }

        // Set the shared index buffer for all quad renderers.
        self.index_buffer
            .set(&mut context.pass, self.max_quads_in_use);

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

        pass.draw_indexed(
            0..(batch.quad_count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
            0,
            0..1,
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
        // Step 1: Get all instance data.
        // OO: Compute a conservative capacity?
        let mut sdf_glyphs = Vec::new();
        let mut color_glyphs = Vec::new();

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

                // OO: translation might be applied to two points only (lt, rb)
                let vertices =
                    Self::glyph_vertices(run, glyph, &placement).map(|p| p + translation);

                match kind {
                    AtlasKind::Sdf => {
                        sdf_glyphs.push(sdf_atlas::QuadInstance {
                            atlas_rect: rect,
                            vertices,
                            // OO: Text color is changing per run only.
                            color: run.text_color,
                        })
                    }
                    AtlasKind::Color => color_glyphs.push(color_atlas::QuadInstance {
                        atlas_rect: rect,
                        vertices,
                    }),
                }
            }
        }

        let sdf_batch = self.sdf_renderer.batch(context, &sdf_glyphs);
        let color_batch = self.color_renderer.batch(context, &color_glyphs);

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

    fn glyph_vertices(
        run: &GlyphRun,
        glyph: &RunGlyph,
        placement: &text::Placement,
    ) -> [Point3; 4] {
        let (lt, rb) = run.place_glyph(glyph, placement);

        // Convert the pixel rect to 3D Points.
        let left = lt.x as f64;
        let top = lt.y as f64;
        let right = rb.x as f64;
        let bottom = rb.y as f64;

        // OO: might use Point3 here.
        let points: [Point; 4] = [
            (left, top).into(),
            (left, bottom).into(),
            (right, bottom).into(),
            (right, top).into(),
        ];

        points.map(|f| f.with_z(0.0))
    }
}
