use std::{collections::HashSet, fmt};

use anyhow::Result;
use cosmic_text::{self as text};
use massive_geometry::{Point, Point3};
use massive_shapes::{GlyphRun, RunGlyph};
use swash::scale::ScaleContext;
use text::SwashContent;
use wgpu::Device;

use crate::{
    FontManager,
    glyph::{
        GlyphRasterizationParam, SwashRasterizationParam, glyph_atlas,
        glyph_rasterization::{RasterizedGlyphKey, rasterize_glyph_with_padding},
    },
    renderer::{PreparationContext, RenderBatch},
    text_layer::{atlas_renderer::AtlasRenderer, color_atlas, sdf_atlas},
};

pub struct TextLayerRenderer {
    // Optimization: This is used for `FontSystem::get_font()` only, which needs &mut. In the long
    // run, completely put the character renderer off-thread and run the rasterizers completely
    // parallel (tokio is probably fine, too). This is needed as soon we need asynchronous
    // optimization of rendered resolutions to match the pixel density.
    fonts: FontManager,
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
            .field("font_system", &self.fonts)
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
    pub fn new(device: &Device, fonts: FontManager, target_format: wgpu::TextureFormat) -> Self {
        Self {
            scale_context: ScaleContext::default(),
            fonts,
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

    /// Prepare a number of glyph runs and produce sdf and color batches.
    ///
    /// All of the runs use the same model matrix.
    pub fn runs_to_batches<'a>(
        &mut self,
        context: &PreparationContext,
        runs: impl Iterator<Item = &'a GlyphRun>,
    ) -> Result<[Option<RenderBatch>; 2]> {
        // Step 1: Get all instance data.
        // Performance: Compute a conservative capacity?
        let mut sdf_glyphs = Vec::new();
        let mut color_glyphs = Vec::new();

        let fonts = self.fonts.clone();

        for run in runs {
            let translation = run.translation;
            for glyph in &run.glyphs {
                let Some((rect, placement, kind)) = self.rasterized_glyph_atlas_rect(
                    context, &fonts, // run.text_weight,
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

        Ok([sdf_batch, color_batch])
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &PreparationContext,
        fonts: &FontManager,
        glyph: &RunGlyph,
    ) -> Result<Option<(glyph_atlas::Rectangle, text::Placement, AtlasKind)>> {
        let glyph_key = RasterizedGlyphKey {
            text: glyph.key,
            param: GlyphRasterizationParam {
                prefer_sdf: true,
                swash: SwashRasterizationParam { hinted: true },
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

        let image = {
            // Architecture: We lock this for the least amount of time, because it might be shared with
            // client application's layout processes (which usually take longer).
            //
            // A better solution would be to build our own Font cache, because it's only used for
            // looking up the Arc<Font> for the glyph using get_font().
            let mut font_system = fonts.lock();
            rasterize_glyph_with_padding(&mut font_system, &mut self.scale_context, &glyph_key)
        };

        let Some(image) = image else {
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

    pub fn create_sdf_pipeline(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        self.sdf_renderer.create_pipeline(device)
    }

    pub fn create_color_pipeline(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        self.color_renderer.create_pipeline(device)
    }
}
