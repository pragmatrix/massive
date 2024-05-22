use std::{collections::HashSet, rc::Rc};

use anyhow::Result;
use cosmic_text as text;
use itertools::Itertools;
use massive_geometry::{Matrix4, Point, Point3};
use massive_shapes::{GlyphRun, GlyphRunShape, RunGlyph, Shape, TextWeight};
use swash::{scale::ScaleContext, Weight};
use wgpu::Device;

use crate::{
    glyph::{
        glyph_atlas, glyph_rasterization::rasterize_padded_glyph, GlyphRasterizationParam,
        RasterizedGlyphKey, SwashRasterizationParam,
    },
    renderer::{PreparationContext, RenderContext},
};

use super::atlas_sdf::{AtlasSdfRenderer, QuadBatch, QuadInstance};

pub struct TextLayerRenderer {
    // Font cache and scratch buffers for the rasterizer.
    //
    // TODO: May make the Rasterizer a thing and put it in there alongside with its functions. This
    // would allow further optimizations I guess (e.g. an own scratch buffer, etc.).
    scale_context: ScaleContext,
    empty_glyphs: HashSet<RasterizedGlyphKey>,

    sdf_renderer: AtlasSdfRenderer,
    sdf_batches: Vec<QuadBatch>,
}

impl TextLayerRenderer {
    pub fn new(
        device: &Device,
        target_format: wgpu::TextureFormat,
        view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            scale_context: ScaleContext::default(),
            empty_glyphs: HashSet::new(),
            sdf_renderer: AtlasSdfRenderer::new(
                device,
                target_format,
                view_projection_bind_group_layout,
            ),
            sdf_batches: Vec::new(),
        }
    }

    pub fn prepare(&mut self, context: &mut PreparationContext, shapes: &[Shape]) -> Result<()> {
        // Group all glyph runs bei their matrix pointer.
        let grouped = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::GlyphRun(shape) => Some(shape),
                _ => None,
            })
            .into_group_map_by(|shape| Rc::as_ptr(&shape.model_matrix));

        self.sdf_batches.clear();

        for (_, shapes) in grouped {
            // NB: could deref the pointer here using unsafe.
            let matrix = &shapes[0].model_matrix;
            if let Some(quad_batch) = self.prepare_runs(context, matrix, &shapes)? {
                self.sdf_batches.push(quad_batch);
            }
        }

        Ok(())
    }

    pub fn render<'rpass>(&'rpass self, context: &mut RenderContext<'_, 'rpass>) {
        self.sdf_renderer.render(context, &self.sdf_batches);
    }

    /// Prepare a number of glyph runs and produce a TextLayer.
    ///
    /// All of the runs use the same model matrix.
    fn prepare_runs(
        &mut self,
        context: &mut PreparationContext,
        model_matrix: &Matrix4,
        // TODO: this double reference is quite unusual here
        shapes: &[&GlyphRunShape],
    ) -> Result<Option<QuadBatch>> {
        // Step 1: Get all instance data.
        // OO: Compute a conservative capacity?
        // OO: We throw this away in this function further down below.
        let mut instances = Vec::new();

        for GlyphRunShape { run, .. } in shapes {
            let translation = run.translation;
            for glyph in &run.glyphs {
                if let Some((rect, placement)) =
                    self.rasterized_glyph_atlas_rect(context, run.text_weight, glyph)?
                {
                    instances.push(QuadInstance {
                        atlas_rect: rect,
                        vertices: Self::glyph_vertices(run, glyph, &placement)
                            // OO: translation might be applied to two points only (lt, rb)
                            .map(|p| p + translation),
                        // OO: Text color is changing per run only.
                        color: run.text_color,
                    })
                } // else: Glyph is empty: Not rendered.
            }
        }

        Ok(if instances.is_empty() {
            None
        } else {
            Some(self.sdf_renderer.batch(context, model_matrix, &instances))
        })
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    //
    // u/v Coordinates are generated later, at a time we know how large the bitmap actually is.
    // TODO: could compute them in the shader, we do have the size of the atlas there.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &mut PreparationContext,
        weight: TextWeight,
        glyph: &RunGlyph,
    ) -> Result<Option<(glyph_atlas::Rectangle, text::Placement)>> {
        let glyph_key = RasterizedGlyphKey {
            text: glyph.key,
            param: GlyphRasterizationParam {
                sdf: true,
                swash: SwashRasterizationParam {
                    hinted: true,
                    weight: Weight(weight.0),
                },
            },
        };

        if let Some((rect, image)) = self.sdf_renderer.atlas.get(&glyph_key) {
            // atlas hit.
            return Ok(Some((rect, image.placement)));
        }

        // atlas / cache miss.

        if self.empty_glyphs.contains(&glyph_key) {
            return Ok(None);
        }

        // not yet in an atlas and not empty.

        let Some(image) =
            rasterize_padded_glyph(context.font_system, &mut self.scale_context, &glyph_key)
        else {
            self.empty_glyphs.insert(glyph_key);
            return Ok(None);
        };

        let image_placement = image.placement;
        let rect_in_atlas =
            self.sdf_renderer
                .atlas
                .store(context.device, context.queue, &glyph_key, image)?;

        Ok(Some((rect_in_atlas, image_placement)))
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
