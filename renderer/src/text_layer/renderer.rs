use std::{
    collections::HashSet,
    fmt,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use massive_geometry::{Point, Point3};
use massive_scene::Matrix;
use massive_shapes::{GlyphRun, RunGlyph, TextWeight};
use swash::{Weight, scale::ScaleContext};
use text::SwashContent;
use wgpu::Device;

use crate::{
    glyph::{
        GlyphRasterizationParam, SwashRasterizationParam, glyph_atlas,
        glyph_rasterization::{RasterizedGlyphKey, rasterize_glyph_with_padding},
    },
    pods::{AsBytes, ToPod},
    renderer::{PipelineBatches, PreparationContext, RenderBatch, RenderContext, RenderVisual},
    scene::LocationMatrices,
    text_layer::{atlas_renderer::AtlasRenderer, color_atlas, sdf_atlas},
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

    sdf_renderer: AtlasRenderer,
    color_renderer: AtlasRenderer,

    /// The maximum quads currently in use. This may be more than the index buffer can hold.
    max_quads_in_use: usize,
}

impl fmt::Debug for TextLayerRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextLayerRenderer")
            .field("font_system", &self.font_system)
            // .field("scale_context", &self.scale_context)
            .field("empty_glyphs", &self.empty_glyphs)
            .field("sdf_renderer", &self.sdf_renderer)
            .field("color_renderer", &self.color_renderer)
            .field("max_quads_in_use", &self.max_quads_in_use)
            .finish()
    }
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
            // Instead of specifying all these consts _and_ the vertex type, a trait based spec type
            // would probably be better.
            sdf_renderer: AtlasRenderer::new::<sdf_atlas::Vertex>(
                device,
                wgpu::TextureFormat::R8Unorm,
                wgpu::include_wgsl!("sdf_atlas.wgsl"),
                target_format,
            ),
            color_renderer: AtlasRenderer::new::<color_atlas::TextureVertex>(
                device,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::include_wgsl!("color_atlas.wgsl"),
                target_format,
            ),
            max_quads_in_use: 0,
        }
    }

    pub fn render_sdf_glyphs<'a>(
        &self,
        matrices: &LocationMatrices,
        visuals: impl Iterator<Item = &'a RenderVisual>,
        context: &mut RenderContext,
    ) {
        context.pass.set_pipeline(self.sdf_renderer.pipeline());

        for visual in visuals {
            if let Some(ref sdf_batch) = visual.batches.sdf {
                let model_matrix = context.pixel_matrix * matrices.get(visual.location_id);
                Self::render_batch(context, &model_matrix, sdf_batch);
            }
        }
    }

    pub fn render_color_glyphs<'a>(
        &self,
        matrices: &LocationMatrices,
        visuals: impl Iterator<Item = &'a RenderVisual>,
        context: &mut RenderContext,
    ) {
        context.pass.set_pipeline(self.color_renderer.pipeline());

        for visual in visuals {
            if let Some(ref color_batch) = visual.batches.color {
                let model_matrix = context.pixel_matrix * matrices.get(visual.location_id);
                Self::render_batch(context, &model_matrix, color_batch);
            }
        }
    }

    pub fn render_batch(context: &mut RenderContext, model_matrix: &Matrix, batch: &RenderBatch) {
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
            0..(batch.count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
            0,
            0..1,
        )
    }

    /// Prepare a number of glyph runs and produce a TextLayer.
    ///
    /// All of the runs use the same model matrix.
    pub fn runs_to_batches<'a>(
        &mut self,
        context: &PreparationContext,
        runs: impl Iterator<Item = &'a GlyphRun>,
    ) -> Result<PipelineBatches> {
        // Step 1: Get all instance data.
        // Performance: Compute a conservative capacity?
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

                // Performance: translation might be applied to two points only (lt, rb).
                // Performance: translation should be in pixel grid space I guess (a i32 tuple?).
                // Otherwise pixel perfect positioning could not be guaranteed. But what about z translations?
                let vertices =
                    Self::glyph_vertices(run, glyph, &placement).map(|p| p + translation);

                match kind {
                    AtlasKind::Sdf => {
                        sdf_glyphs.push(sdf_atlas::Instance {
                            atlas_rect: rect,
                            vertices,
                            // OO: Text color is changing per run only.
                            color: run.text_color,
                        })
                    }
                    AtlasKind::Color => color_glyphs.push(color_atlas::Instance {
                        atlas_rect: rect,
                        vertices,
                    }),
                }
            }
        }

        let sdf_batch = self.sdf_renderer.batch(context, &sdf_glyphs);
        let color_batch = self.color_renderer.batch(context, &color_glyphs);

        Ok(PipelineBatches {
            sdf: sdf_batch,
            color: color_batch,
            shapes: None,
        })
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &PreparationContext,
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
