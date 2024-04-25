//! A shape to primitive renderer that produces text layers.

use std::rc::Rc;

use cosmic_text as text;
use itertools::Itertools;
use massive_geometry::Matrix4;
use massive_shapes::Shape;
use swash::shape::cluster::Glyph;
use tracing::instrument;
use wgpu::Device;

use crate::{glyph::GlyphAtlas, primitives::Primitive, text_layer, tools::texture_sampler};

struct ShapeLayerRendererContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub text_layer_bind_group_layout: &'a text_layer::BindGroupLayout,
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
            text_layer_bind_group_layout,
            font_system,
        }
    }
}

pub struct ShapeLayerRenderer {
    texture_sampler: wgpu::Sampler,
    atlas: GlyphAtlas,
}

impl ShapeLayerRenderer {
    pub fn new(device: &Device) -> Self {
        Self {
            texture_sampler: texture_sampler::linear_clamping(device),
            atlas: GlyphAtlas::new(device),
        }
    }

    #[instrument(skip_all)]
    pub fn render(
        &mut self,
        context: &mut ShapeLayerRendererContext,
        shapes: &[Shape],
    ) -> Vec<Primitive> {
        // Group all glyph runs bei their matrix pointer.
        let grouped = shapes.iter().into_group_map_by(|shape| {
            let Shape::GlyphRun { model_matrix, .. } = shape;
            Rc::as_ptr(model_matrix)
        });

        let primitives = Vec::with_capacity(grouped.len());
        for (_, shapes) in grouped {
            // NB: could deref the pointer here using unsafe.
            let matrix = {
                let Shape::GlyphRun { model_matrix, .. } = shapes[0];
                model_matrix
            };
            let primitive = self.render_runs(context, matrix, &shapes);
            // primitives.push(primitive);
        }
        primitives
    }

    /// Render a number of glyph runs into one TextLayer.
    pub fn render_runs(
        &mut self,
        context: &mut ShapeLayerRendererContext,
        matrix: &Matrix4,
        // TODO: this double reference is quite unusual here
        shapes: &[&Shape],
    ) -> () {
        // let mut vertices = Vec::new();
        // let mut instances = Vec::new();

        for shape in shapes {
            let Shape::GlyphRun {
                translation, run, ..
            } = shape;

            for glyph in &run.glyphs {
                // let glyph_key = glyph.key(false)
            }
        }
    }
}
