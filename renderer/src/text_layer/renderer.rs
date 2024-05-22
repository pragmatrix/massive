use std::{collections::HashSet, rc::Rc};

use anyhow::Result;
use itertools::Itertools;
use massive_geometry::{Color, Matrix4, Point, Point3};
use massive_shapes::{GlyphRun, GlyphRunShape, RunGlyph, Shape, TextWeight};
use swash::{scale::ScaleContext, Weight};
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    BufferUsages, Device,
};

use super::BindGroupLayout;
use crate::{
    glyph::{
        glyph_atlas, glyph_rasterization::rasterize_padded_glyph, GlyphAtlas,
        GlyphRasterizationParam, RasterizedGlyphKey, SwashRasterizationParam,
    },
    pods::TextureColorVertex,
    renderer::{PreparationContext, RenderContext},
    text,
    tools::{create_pipeline, texture_sampler, QuadIndexBuffer},
    SizeBuffer,
};

pub struct TextLayerRenderer {
    // Font cache and scratch buffers for the rasterizer.
    //
    // TODO: May make the Rasterizer a thing and put it in there alongside with its functions. This
    // would allow further optimizations I guess (e.g. an own scratch buffer, etc.).
    scale_context: ScaleContext,
    atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    fs_bind_group_layout: BindGroupLayout,
    index_buffer: QuadIndexBuffer,

    empty_glyphs: HashSet<RasterizedGlyphKey>,

    layers: Vec<TextLayer>,
}

/// A layer of 3D text backed by a texture atlas.
struct TextLayer {
    // Matrix is not supplied as a buffer, because it is combined with the camera matrix before
    // uploading to the shader.
    model_matrix: Matrix4,
    fs_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    quad_count: usize,
}

impl TextLayerRenderer {
    pub fn new(
        device: &Device,
        target_format: wgpu::TextureFormat,
        view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = &device.create_shader_module(wgpu::include_wgsl!("text_layer.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Text Layer Pipeline Layout"),
            bind_group_layouts: &[view_projection_bind_group_layout, &fs_bind_group_layout],
            push_constant_ranges: &[],
        });

        let targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_layout = [TextureColorVertex::layout()];

        let pipeline = create_pipeline(
            "Text Layer Pipeline",
            device,
            shader,
            "fs_sdf_glyph",
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            scale_context: ScaleContext::default(),
            atlas: GlyphAtlas::new(device),
            texture_sampler: texture_sampler::linear_clamping(device),
            fs_bind_group_layout,
            pipeline,
            index_buffer: QuadIndexBuffer::new(device),
            empty_glyphs: HashSet::default(),
            layers: Vec::new(),
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

        self.layers.clear();
        if grouped.len() > self.layers.len() {
            self.layers.reserve(grouped.len() - self.layers.len())
        }

        let mut max_quads = 0;

        for (_, shapes) in grouped {
            // NB: could deref the pointer here using unsafe.
            let matrix = &shapes[0].model_matrix;
            if let Some(text_layer) = self.prepare_runs(context, matrix, &shapes)? {
                max_quads = max_quads.max(text_layer.quad_count);
                self.layers.push(text_layer)
            }
        }

        self.index_buffer
            .ensure_can_index_num_quads(context.device, max_quads);

        Ok(())
    }

    pub fn render<'rpass>(&'rpass self, context: &mut RenderContext<'_, 'rpass>) {
        let pass = &mut context.pass;
        pass.set_pipeline(&self.pipeline);
        // DI: May do this inside this renderer and pass a Matrix to prepare?.
        pass.set_bind_group(0, context.view_projection_bind_group, &[]);
        // DI: May share index buffers between renderers?
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        for TextLayer {
            model_matrix,
            fs_bind_group,
            vertex_buffer,
            quad_count,
        } in &self.layers
        {
            let text_layer_matrix = context.view_projection_matrix * model_matrix;

            // OO: Set bind group only once and update the buffer?
            context.queue_view_projection_matrix(&text_layer_matrix);

            let pass = &mut context.pass;
            pass.set_bind_group(0, context.view_projection_bind_group, &[]);

            pass.set_bind_group(1, fs_bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));

            pass.draw_indexed(
                0..(quad_count * QuadIndexBuffer::QUAD_INDICES_COUNT) as u32,
                0,
                0..1,
            )
        }
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
    ) -> Result<Option<TextLayer>> {
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
                    instances.push(GlyphInstance {
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
            // This also would prevent us here from a second loop and storing the vertices (because
            // atlas size is known after all glyphs are rasterized)
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

        let bind_group = self.fs_bind_group_layout.create_bind_group(
            context.device,
            self.atlas.texture_view(),
            &texture_size,
            &self.texture_sampler,
        );

        let text_layer = TextLayer {
            model_matrix: *model_matrix,
            fs_bind_group: bind_group,
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
