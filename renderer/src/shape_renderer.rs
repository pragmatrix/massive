//! Converts shapes to primitives.
//!
//! Primitives are lower-level constructs that contain references to wgpu Resources.

use cgmath::{Point2, Transform};
use cosmic_text as text;
use tracing::instrument;
use wgpu::Device;

use crate::{
    glyph::{GlyphCache, GlyphClass, GlyphRenderParam, RenderGlyphKey},
    primitives::Primitive,
    texture::{self, Texture},
    tools::texture_sampler,
    ColorBuffer,
};
use massive_geometry::{Matrix4, Point};
use massive_shapes::{GlyphRun, PositionedGlyph, Shape};

pub struct ShapeRenderer {
    texture_sampler: wgpu::Sampler,
    glyph_cache: GlyphCache,
}

pub struct ShapeRendererContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub texture_bind_group_layout: &'a texture::BindGroupLayout,
    pub font_system: &'a mut text::FontSystem,
}

impl<'a> ShapeRendererContext<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        texture_bind_group_layout: &'a texture::BindGroupLayout,
        font_system: &'a mut text::FontSystem,
    ) -> Self {
        Self {
            device,
            queue,
            texture_bind_group_layout,
            font_system,
        }
    }
}

impl ShapeRenderer {
    pub fn new(device: &Device) -> Self {
        Self {
            texture_sampler: texture_sampler::linear_clamping(device),
            glyph_cache: GlyphCache::default(),
        }
    }

    #[instrument(skip_all)]
    pub fn render(
        &mut self,
        context: &mut ShapeRendererContext,
        // Needed to compute pixel / planar classification.
        surface_view_matrix: &Matrix4,
        shapes: &[Shape],
    ) -> Vec<Primitive> {
        let primitives = shapes
            .iter()
            .flat_map(|shape| self.render_shape(context, surface_view_matrix, shape))
            .collect();

        self.glyph_cache.flush_unused();

        primitives
    }

    // TODO: Prevent excessive Vec<Primitive> allocation.
    fn render_shape(
        &mut self,
        context: &mut ShapeRendererContext,
        surface_view_matrix: &Matrix4,
        shape: &Shape,
    ) -> Vec<Primitive> {
        match shape {
            Shape::GlyphRun {
                model_matrix,
                translation,
                run,
            } => self.render_glyph_run(
                context,
                surface_view_matrix,
                &(**model_matrix * Matrix4::from_translation(*translation)),
                run,
            ),
        }
    }

    fn render_glyph_run(
        &mut self,
        context: &mut ShapeRendererContext,
        surface_view_matrix: &Matrix4,
        model_matrix: &Matrix4,
        glyph_run: &GlyphRun,
    ) -> Vec<Primitive> {
        // TODO: cache this.
        let surface_run_matrix = surface_view_matrix * *model_matrix;

        let text_color_buffer = ColorBuffer::new(context.device, glyph_run.text_color);

        glyph_run
            .glyphs
            .iter()
            .filter_map(|glyph| {
                self.render_glyph(
                    context,
                    model_matrix,
                    glyph_run,
                    &text_color_buffer,
                    &surface_run_matrix,
                    glyph,
                )
            })
            .collect()
    }

    #[tracing::instrument(skip_all)]
    fn render_glyph(
        &mut self,
        context: &mut ShapeRendererContext,
        model_matrix: &Matrix4,
        run: &GlyphRun,
        color_buffer: &ColorBuffer,
        glyph_to_surface: &Matrix4,
        glyph: &PositionedGlyph,
    ) -> Option<Primitive> {
        let metrics = run.metrics;
        // Compute the bounds of a pixel in the middle of the glyph (in glyph pixel coordinates)
        let pixel_bounds = {
            let (_, height) = metrics.size();
            // TODO: we might pull this up to the center of the part of the glyph above the
            // baseline.
            let half_height = height / 2;
            let x = (glyph.hitbox_width as u32) / 2;
            glyph.pixel_bounds_at((x, half_height))
        };

        // Transform the points of the bounds to the surface texture coordinate system.
        let surface_points = pixel_bounds
            .to_rect()
            .to_quad()
            .map(|p| p.with_z(0.0))
            .map(|p| glyph_to_surface.transform_point(p));

        // Classify
        let class = GlyphClass::from_transformed_pixel(&surface_points);

        let render_param: GlyphRenderParam = class.into();
        let pipeline = render_param.pipeline();

        let render_glyph = self.glyph_cache.get(
            context.device,
            context.queue,
            context.font_system,
            RenderGlyphKey {
                text: glyph.key,
                param: render_param,
            },
        )?;

        // TODO: Need a i32 and f32 2D Rect here.

        let (lt, rb) = place_glyph(
            run.metrics.max_ascent,
            glyph.hitbox_pos,
            render_glyph.placement,
        );

        // Convert the pixel rect 3D Points.
        let points = {
            let left = lt.x as f64;
            let top = lt.y as f64;
            let right = rb.x as f64;
            let bottom = rb.y as f64;

            // TODO: might use Point3 here.
            let points: [Point; 4] = [
                (left, top).into(),
                (left, bottom).into(),
                (right, bottom).into(),
                (right, top).into(),
            ];

            points.map(|f| f.with_z(0.0))
        };

        // Transform them with the pixel / model matrix.

        let transformed = points.map(|p| model_matrix.transform_point(p));

        let texture = Texture::new(
            context.device,
            pipeline,
            context.texture_bind_group_layout,
            &self.texture_sampler,
            &render_glyph.texture_view,
            color_buffer,
            &transformed,
        );

        Some(Primitive::Texture(texture))
    }
}

/// TODO: put this to GlyphRunMetrics
fn place_glyph(
    max_ascent: u32,
    hitbox_pos: (i32, i32),
    placement: text::Placement,
) -> (Point2<i32>, Point2<i32>) {
    let left = hitbox_pos.0 + placement.left;
    let top = hitbox_pos.1 + (max_ascent as i32) - placement.top;
    let right = left + placement.width as i32;
    let bottom = top + placement.height as i32;

    ((left, top).into(), (right, bottom).into())
}
