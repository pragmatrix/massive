use massive_scene::Matrix;
use wgpu::{
    TextureFormat,
    util::{BufferInitDescriptor, DeviceExt},
};

use crate::{
    glyph::GlyphAtlas,
    pods::TextureVertex,
    renderer::{PreparationContext, RenderContext},
    text_layer::{QuadBatch, color_atlas::QuadInstance},
    tools::{QuadIndexBuffer, create_pipeline, texture_sampler},
};

use super::BindGroupLayout;

pub struct ColorAtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    fs_bind_group_layout: BindGroupLayout,
}

impl ColorAtlasRenderer {
    pub fn new(
        device: &wgpu::Device,
        target_format: TextureFormat,
        view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = &device.create_shader_module(wgpu::include_wgsl!("color_atlas.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atlas SDF Pipeline Layout"),
            bind_group_layouts: &[view_projection_bind_group_layout, &fs_bind_group_layout],
            push_constant_ranges: &[],
        });

        let targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_layout = [TextureVertex::layout()];

        let pipeline = create_pipeline(
            "Color Atlas Pipeline",
            device,
            shader,
            "fs_color",
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            atlas: GlyphAtlas::new(device, TextureFormat::Rgba8Unorm),
            texture_sampler: texture_sampler::linear_clamping(device),
            fs_bind_group_layout,
            pipeline,
        }
    }

    // Convert a number of instances to a batch.
    pub fn batch(
        &mut self,
        context: &PreparationContext,
        instances: &[QuadInstance],
    ) -> Option<QuadBatch> {
        if instances.is_empty() {
            return None;
        }
        let mut vertices = Vec::with_capacity(instances.len() * 4);

        for instance in instances {
            let r = instance.atlas_rect;
            // ADR: u/v normalization is dont in the shader, for once, its probably free, and
            // secondly we don't have to care about the atlas texture growing as long the rects stay
            // the same.
            let (ltx, lty) = (r.min.x as f32, r.min.y as f32);
            let (rbx, rby) = (r.max.x as f32, r.max.y as f32);

            let v = &instance.vertices;
            vertices.extend([
                TextureVertex::new(v[0], (ltx, lty)),
                TextureVertex::new(v[1], (ltx, rby)),
                TextureVertex::new(v[2], (rbx, rby)),
                TextureVertex::new(v[3], (rbx, lty)),
            ]);
        }

        let device = context.device;

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Text Layer Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let bind_group = self.fs_bind_group_layout.create_bind_group(
            device,
            self.atlas.texture_view(),
            &self.texture_sampler,
        );

        Some(QuadBatch {
            fs_bind_group: bind_group,
            vertex_buffer,
            quad_count: instances.len(),
        })
    }

    pub fn prepare(&self, context: &mut RenderContext) {
        let pass = &mut context.pass;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, context.view_projection_bind_group, &[]);
    }

    pub fn render(&self, context: &mut RenderContext, model_matrix: &Matrix, batch: &QuadBatch) {
        let text_layer_matrix = context.view_projection_matrix * model_matrix;

        context.queue_view_projection_matrix(&text_layer_matrix);

        let pass = &mut context.pass;

        pass.set_bind_group(1, &batch.fs_bind_group, &[]);
        pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));

        pass.draw_indexed(
            0..(batch.quad_count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
            0,
            0..1,
        )
    }
}
