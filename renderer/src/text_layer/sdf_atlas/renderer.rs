use wgpu::util::{BufferInitDescriptor, DeviceExt};

use super::BindGroupLayout;
use crate::{
    glyph::GlyphAtlas,
    pods::{self, AsBytes, TextureColorVertex, ToPod},
    renderer::{PreparationContext, RenderContext},
    text_layer::{QuadBatch, sdf_atlas::QuadInstance},
    tools::{QuadIndexBuffer, create_pipeline, texture_sampler},
};
use massive_scene::Matrix;

pub struct SdfAtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    fs_bind_group_layout: BindGroupLayout,
}

impl SdfAtlasRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = &device.create_shader_module(wgpu::include_wgsl!("sdf_atlas.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atlas SDF Pipeline Layout"),
            bind_group_layouts: &[&fs_bind_group_layout],
            push_constant_ranges: &[wgpu::PushConstantRange {
                stages: wgpu::ShaderStages::VERTEX,
                range: 0..pods::Matrix4::size(),
            }],
        });

        let targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_layout = [TextureColorVertex::layout()];

        let pipeline = create_pipeline(
            "Atlas SDF Pipeline",
            device,
            shader,
            "fs_sdf",
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            atlas: GlyphAtlas::new(device, wgpu::TextureFormat::R8Unorm),
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
            // ADR: u/v normalization is done in the shader, because its probably free and we don't
            // have to care about the atlas texture growing as long the rects stay the same.
            let (ltx, lty) = (r.min.x as f32, r.min.y as f32);
            let (rbx, rby) = (r.max.x as f32, r.max.y as f32);

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

        let bind_group = self.fs_bind_group_layout.create_bind_group(
            context.device,
            self.atlas.texture_view(),
            &self.texture_sampler,
        );

        // Grow index buffer as needed.

        let quad_count = instances.len();

        Some(QuadBatch {
            fs_bind_group: bind_group,
            vertex_buffer,
            quad_count,
        })
    }

    // Architecture: Return some struct / context that enables repeated render(matrix, batch)
    // invocations.
    pub fn prepare(&self, context: &mut RenderContext) {
        let pass = &mut context.pass;
        pass.set_pipeline(&self.pipeline);
    }

    /// The prerequisites for calling render():
    /// - A prepared quad index buffer
    /// - prepare() invocation.
    pub fn render(&self, context: &mut RenderContext, model_matrix: &Matrix, batch: &QuadBatch) {
        let text_layer_matrix = context.view_projection_matrix * model_matrix;

        let pass = &mut context.pass;

        pass.set_push_constants(
            wgpu::ShaderStages::VERTEX,
            0,
            text_layer_matrix.to_pod().as_bytes(),
        );
        // Also, this is mostly the same, set this only once.
        pass.set_bind_group(0, &batch.fs_bind_group, &[]);
        pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));

        pass.draw_indexed(
            0..(batch.quad_count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
            0,
            0..1,
        )
    }
}
