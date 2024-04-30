//! A shape to primitive renderer that produces text layers.

use std::{collections::HashSet, rc::Rc};

use anyhow::Result;
use cosmic_text as text;
use itertools::Itertools;
use massive_geometry::{Color, Matrix4, Point, Point3};
use massive_shapes::{GlyphRun, RunGlyph, Shape};
use swash::scale::ScaleContext;
use tracing::instrument;
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    BufferUsages, Device,
};

use crate::{
    glyph::{
        glyph_atlas, glyph_rasterization::rasterize_padded_glyph, GlyphAtlas,
        GlyphRasterizationParam, RasterizedGlyphKey,
    },
    pods::{InstanceColor, TextureColorVertex, TextureVertex},
    primitives::Primitive,
    text_layer::{self, TextLayer},
    tools::texture_sampler,
    SizeBuffer,
};

pub struct ShapeLayerRendererContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub bind_group_layout: &'a text_layer::BindGroupLayout,
    pub font_system: &'a mut text::FontSystem,
}

impl<'a> ShapeLayerRendererContext<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        text_layer_bind_group_layout: &'a text_layer::BindGroupLayout,
        font_system: &'a mut text::FontSystem,
    ) -> Self {
        Self {
            device,
            queue,
            bind_group_layout: text_layer_bind_group_layout,
            font_system,
        }
    }
}

pub struct ShapeLayerRenderer {
    // Font cache and scratch buffers for the rasterizer.
    //
    // TODO: May make the Rasterizer a thing and put it in there alongside with its functions. This
    // would allow further optimizations I guess (e.g. an own scratch buffer, etc.).
    scale_context: ScaleContext,
    atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    empty_glyphs: HashSet<RasterizedGlyphKey>,
}

impl ShapeLayerRenderer {
    pub fn new(device: &Device) -> Self {
        Self {
            scale_context: ScaleContext::default(),
            atlas: GlyphAtlas::new(device),
            texture_sampler: texture_sampler::linear_clamping(device),
            empty_glyphs: HashSet::default(),
        }
    }

    #[instrument(skip_all)]
    pub fn render(
        &mut self,
        context: &mut ShapeLayerRendererContext,
        shapes: &[Shape],
    ) -> Result<Vec<Primitive>> {
        // Group all glyph runs bei their matrix pointer.
        let grouped = shapes.iter().into_group_map_by(|shape| {
            let Shape::GlyphRun { model_matrix, .. } = shape;
            Rc::as_ptr(model_matrix)
        });

        let mut primitives = Vec::with_capacity(grouped.len());
        for (_, shapes) in grouped {
            // NB: could deref the pointer here using unsafe.
            let matrix = {
                let Shape::GlyphRun { model_matrix, .. } = shapes[0];
                model_matrix
            };
            if let Some(text_layer) = self.render_runs(context, matrix, &shapes)? {
                primitives.push(Primitive::TextLayer(text_layer))
            }
        }
        Ok(primitives)
    }

    /// Render a number of glyph runs into one TextLayer.
    /// All of the runs use the same model matrix.
    pub fn render_runs(
        &mut self,
        context: &mut ShapeLayerRendererContext,
        model_matrix: &Matrix4,
        // TODO: this double reference is quite unusual here
        shapes: &[&Shape],
    ) -> Result<Option<TextLayer>> {
        // Step 1: Get all instance data.
        // OO: Compute a conservative capacity?
        // OO: We throw this away in this function further down below.
        let mut instances = Vec::new();

        for shape in shapes {
            let Shape::GlyphRun {
                translation: run_translation,
                run,
                ..
            } = shape;

            for glyph in &run.glyphs {
                if let Some((rect, placement)) = self.rasterized_glyph_atlas_rect(context, glyph)? {
                    instances.push(GlyphInstance {
                        atlas_rect: rect,
                        vertices: Self::glyph_vertices(run, glyph, &placement)
                            // OO: translation might be applied to two points only (lt, rb)
                            .map(|p| p + run_translation),
                        // OO: Text color is changing per run only.
                        color: run.text_color,
                    })
                } // else: Glyph is empty: Not rendered.
            }
        }

        if instances.is_empty() {
            return Ok(None);
        }

        // Convert instances to buffers.

        let atlas_texture_size = self.atlas.size();

        // Prepare u/v normalization.
        let (to_uv_h, to_uv_v) = {
            let (width, height) = atlas_texture_size;
            ((1.0 / width as f32), (1.0 / height as f32))
        };

        let mut vertices = Vec::with_capacity(instances.len() * 4);

        for instance in &instances {
            let r = instance.atlas_rect;
            // OO: We could do u/v normalization in the shader, the atlas texture size is known.
            let (ltx, lty) = (r.min.x as f32 * to_uv_h, r.min.y as f32 * to_uv_v);
            let (rbx, rby) = (r.max.x as f32 * to_uv_h, r.max.y as f32 * to_uv_v);

            let v = &instance.vertices;
            let color = instance.color;
            vertices.extend([
                TextureColorVertex::new(v[0], (ltx, lty), color),
                TextureColorVertex::new(v[1], (ltx, rby), color),
                TextureColorVertex::new(v[2], (rbx, rby), color),
                TextureColorVertex::new(v[3], (rbx, lty), color),
            ]);
        }

        let device = context.device;

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Text Layer Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        });

        // OO: Let atlas maintain this one, so that's only regenerated when it grows?
        let texture_size = SizeBuffer::new(device, atlas_texture_size);

        let bind_group = context.bind_group_layout.create_bind_group(
            context.device,
            self.atlas.texture_view(),
            &texture_size,
            &self.texture_sampler,
        );

        let text_layer = TextLayer {
            model_matrix: *model_matrix,
            fragment_shader_bind_group: bind_group,
            vertex_buffer,
            quad_count: instances.len(),
        };

        Ok(Some(text_layer))
    }

    // This makes sure that there is a rasterized glyph in the atlas and returns the rectangle.
    //
    // u/v Coordinates are generated later, at a time we know how large the bitmap actually is.
    // TODO: could compute them in the shader, we do have the size of the atlas there.
    fn rasterized_glyph_atlas_rect(
        &mut self,
        context: &mut ShapeLayerRendererContext,
        glyph: &RunGlyph,
    ) -> Result<Option<(glyph_atlas::Rectangle, text::Placement)>> {
        let glyph_key = RasterizedGlyphKey {
            text: glyph.key,
            param: GlyphRasterizationParam {
                sdf: true,
                hinted: true,
            },
        };

        if let Some((rect, image)) = self.atlas.get(&glyph_key) {
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
        let rect_in_atlas = self
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

#[derive(Debug)]
struct GlyphInstance {
    atlas_rect: glyph_atlas::Rectangle,
    vertices: [Point3; 4],
    color: Color,
}
