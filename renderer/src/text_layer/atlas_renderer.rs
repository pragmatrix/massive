use bytemuck::Pod;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::glyph::GlyphAtlas;
use crate::pods::{self, AsBytes, VertexLayout};
use crate::renderer::{PreparationContext, RenderBatch};
use crate::tools::{PipelineParams, PipelineVariant, TextureBindGroupLayout, texture_sampler};

const FRAGMENT_SHADER_ENTRY: &str = "fs_main";

#[derive(Debug)]
pub struct AtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline_params: PipelineParams,
    fs_bind_group_layout: TextureBindGroupLayout,
}

impl AtlasRenderer {
    pub fn new<VertexT: VertexLayout>(
        device: &wgpu::Device,
        atlas_format: wgpu::TextureFormat,
        shader: wgpu::ShaderModuleDescriptor<'_>,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let fs_bind_group_layout = TextureBindGroupLayout::new(device);

        let shader = device.create_shader_module(shader);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atlas Pipeline Layout"),
            bind_group_layouts: &[Some(&*fs_bind_group_layout)],
            immediate_size: pods::Immediates::size(),
        });

        let targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_layout = [VertexT::layout()];

        Self {
            atlas: GlyphAtlas::new(device, atlas_format),
            texture_sampler: texture_sampler::linear_clamping(device),
            fs_bind_group_layout,
            pipeline_params: PipelineParams {
                shader,
                pipeline_layout,
                targets,
                vertex_layout,
            },
        }
    }

    pub fn create_pipeline(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> wgpu::RenderPipeline {
        self.pipeline_params.create_pipeline(
            "Atlas Pipeline",
            device,
            FRAGMENT_SHADER_ENTRY,
            variant,
        )
    }

    // Convert a number of instances to a batch.
    pub fn batch<InstanceT: AtlasInstance>(
        &self,
        context: &PreparationContext,
        instances: &[InstanceT],
    ) -> Option<RenderBatch> {
        if instances.is_empty() {
            return None;
        }
        let mut vertices = Vec::with_capacity(instances.len() * 4);

        for instance in instances {
            vertices.extend(instance.to_vertices());
        }

        let device = context.device;

        let fs_bind_group = self.fs_bind_group_layout.create_bind_group(
            device,
            self.atlas.texture_view(),
            &self.texture_sampler,
        );

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Atlas Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(RenderBatch::single(
            vertex_buffer,
            Some(fs_bind_group),
            instances.len(),
        ))
    }
}

pub trait AtlasInstance {
    type Vertex: Pod;

    fn to_vertices(&self) -> [Self::Vertex; 4];
}
