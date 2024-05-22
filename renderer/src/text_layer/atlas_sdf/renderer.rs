use massive_geometry::{Color, Matrix4, Point3};
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use super::BindGroupLayout;
use crate::{
    glyph::{glyph_atlas, GlyphAtlas},
    pods::TextureColorVertex,
    renderer::{PreparationContext, RenderContext},
    tools::{create_pipeline, texture_sampler, QuadIndexBuffer},
    SizeBuffer,
};

pub struct AtlasSdfRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    fs_bind_group_layout: BindGroupLayout,
    // OO: Share this sucker.
    index_buffer: QuadIndexBuffer,
}

pub struct QuadBatch {
    // Matrix is not prepared as a buffer, because it is combined with the camera matrix before
    // uploading to the shader.
    model_matrix: Matrix4,
    fs_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    quad_count: usize,
}

#[derive(Debug)]
pub struct QuadInstance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Point3; 4],
    pub color: Color,
}

impl AtlasSdfRenderer {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = &device.create_shader_module(wgpu::include_wgsl!("atlas_sdf.wgsl"));

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
            "fs_sdf",
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            atlas: GlyphAtlas::new(device),
            texture_sampler: texture_sampler::linear_clamping(device),
            fs_bind_group_layout,
            pipeline,
            index_buffer: QuadIndexBuffer::new(device),
        }
    }

    // Convert a number of instances to a batch.
    pub fn batch(
        &mut self,
        context: &PreparationContext,
        model_matrix: &Matrix4,
        instances: &[QuadInstance],
    ) -> QuadBatch {
        let atlas_texture_size = self.atlas.size();

        // Prepare u/v normalization.
        let (to_uv_h, to_uv_v) = {
            let (width, height) = atlas_texture_size;
            ((1.0 / width as f32), (1.0 / height as f32))
        };

        let mut vertices = Vec::with_capacity(instances.len() * 4);

        for instance in instances {
            let r = instance.atlas_rect;
            // OO: We could do u/v normalization in the shader, the atlas texture size is known.
            // This also would prevent us here from a second loop and storing the vertices (because
            // atlas size is known after all glyphs are rasterized)
            // TODO: There is even a bug here: the atlas may grow further with each batch.
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
            usage: wgpu::BufferUsages::VERTEX,
        });

        // OO: Let atlas maintain this one, so that's only regenerated when it grows?
        let texture_size = SizeBuffer::new(device, atlas_texture_size);

        let bind_group = self.fs_bind_group_layout.create_bind_group(
            context.device,
            self.atlas.texture_view(),
            &texture_size,
            &self.texture_sampler,
        );

        // Grow index buffer as needed.

        let quad_count = instances.len();
        self.index_buffer
            .ensure_can_index_num_quads(context.device, quad_count);

        QuadBatch {
            model_matrix: *model_matrix,
            fs_bind_group: bind_group,
            vertex_buffer,
            quad_count,
        }
    }

    pub fn render<'rpass>(
        &'rpass self,
        context: &mut RenderContext<'_, 'rpass>,
        batches: &'rpass [QuadBatch],
    ) {
        let pass = &mut context.pass;
        pass.set_pipeline(&self.pipeline);
        // DI: May do this inside this renderer and pass a Matrix to prepare?.
        pass.set_bind_group(0, context.view_projection_bind_group, &[]);
        // DI: May share index buffers between renderers?
        //
        // OO: Don't pass the full index buffer here, only what's actully needed (it is growing
        // only)
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        for QuadBatch {
            model_matrix,
            fs_bind_group,
            vertex_buffer,
            quad_count,
        } in batches
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
}
