use std::collections::HashSet;

use anyhow::Result;
use cosmic_text as text;
use massive_geometry::{Matrix4, Point, Point3};
use massive_scene::Shape;
use massive_shapes::{GlyphRun, RunGlyph, TextWeight};
use swash::{scale::ScaleContext, Weight};
use text::SwashContent;
use wgpu::Device;

use super::{
    color_atlas::{self, ColorAtlasRenderer},
    sdf_atlas::{self, SdfAtlasRenderer},
};
use crate::{
    glyph::{
        glyph_atlas, glyph_rasterization::rasterize_glyph_with_padding, GlyphRasterizationParam,
        RasterizedGlyphKey, SwashRasterizationParam,
    },
    renderer::{PreparationContext, RenderContext},
};

pub struct TextLayerRenderer {
    // Font cache and scratch buffers for the rasterizer.
    //
    // TODO: May make the Rasterizer a thing and put it in there alongside with its functions. This
    // would allow further optimizations I guess (e.g. an own scratch buffer, etc.).
    scale_context: ScaleContext,
    empty_glyphs: HashSet<RasterizedGlyphKey>,

    sdf_renderer: SdfAtlasRenderer,
    sdf_batches: Vec<sdf_atlas::QuadBatch>,

    color_renderer: ColorAtlasRenderer,
    color_batches: Vec<color_atlas::QuadBatch>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum AtlasKind {
    Sdf,
    Color,
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

            sdf_renderer: SdfAtlasRenderer::new(
                device,
                target_format,
                view_projection_bind_group_layout,
            ),
            sdf_batches: Vec::new(),

            color_renderer: ColorAtlasRenderer::new(
                device,
                target_format,
                view_projection_bind_group_layout,
            ),
            color_batches: Vec::new(),
        }
    }

    pub fn prepare(
        &mut self,
        context: &mut PreparationContext,
        shapes: &[(&Matrix4, &[&Shape])],
    ) -> Result<()> {
        // Group all glyph runs bei their matrix pointer.

        self.sdf_batches.clear();
        self.color_batches.clear();

        for (matrix, shapes) in shapes {
            // NB: could deref the pointer here using unsafe.
            let (sdf_batch, color_batch) = self.prepare_runs(
                context,
                matrix,
                // DI: Move this filter up (callers should just pass here what's needed).
                shapes.iter().filter_map(|s| match s {
                    Shape::GlyphRun(run) => Some(run),
                    Shape::Quads(_) => None,
                }),
            )?;
            self.sdf_batches.extend(sdf_batch.into_iter());
            self.color_batches.extend(color_batch.into_iter());
        }

        Ok(())
    }

    pub fn render<'rpass>(&'rpass self, context: &mut RenderContext<'_, 'rpass>) {
        self.sdf_renderer.render(context, &self.sdf_batches);
        self.color_renderer.render(context, &self.color_batches);
    }

    /// Prepare a number of glyph runs and produce a TextLayer.
    ///
    /// All of the runs use the same model matrix.
    fn prepare_runs<'a>(
        &mut self,
        context: &mut PreparationContext,
        model_matrix: &Matrix4,
        // TODO: this double reference is quite unusual here
        runs: impl Iterator<Item = &'a GlyphRun>,
    ) -> Result<(Option<sdf_atlas::QuadBatch>, Option<color_atlas::QuadBatch>)> {
        // Step 1: Get all instance data.
        // OO: Compute a conservative capacity?
        let mut sdf_glyphs = Vec::new();
        let mut color_glyphs = Vec::new();

        for run in runs {
            let translation = run.translation;
            for glyph in &run.glyphs {
                if let Some((rect, placement, kind)) =
                    self.rasterized_glyph_atlas_rect(context, run.text_weight, glyph)?
                {
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
                            vertices: Self::glyph_vertices(run, glyph, &placement)
                                // OO: translation might be applied to two points only (lt, rb)
                                .map(|p| p + translation),
                        }),
                    }
                } // else: Glyph is empty: Not rendered.
            }
        }

        let sdf_batch = self.sdf_renderer.batch(context, model_matrix, &sdf_glyphs);

        let color_batch = self
            .color_renderer
            .batch(context, model_matrix, &color_glyphs);

        Ok((sdf_batch, color_batch))
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &mut PreparationContext,
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

        if let Some((rect, image)) = self.sdf_renderer.atlas.get(&glyph_key) {
            return Ok(Some((rect, image.placement, AtlasKind::Sdf)));
        }

        if let Some((rect, image)) = self.color_renderer.atlas.get(&glyph_key) {
            return Ok(Some((rect, image.placement, AtlasKind::Color)));
        }

        // Atlas / cache miss, empty cached glyph?.
        if self.empty_glyphs.contains(&glyph_key) {
            return Ok(None);
        }

        // Not yet in an atlas and not empty. Now rasterize.
        let Some(image) =
            rasterize_glyph_with_padding(context.font_system, &mut self.scale_context, &glyph_key)
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
