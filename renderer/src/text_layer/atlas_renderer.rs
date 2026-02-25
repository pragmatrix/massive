use bytemuck::Pod;
use derive_more::Deref;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::{
    bind_group_entries,
    glyph::GlyphAtlas,
    pods::{self, AsBytes, VertexLayout},
    renderer::{PreparationContext, RenderBatch},
    tools::{BindGroupLayoutBuilder, PipelineParams, PipelineVariant, texture_sampler},
};

const FRAGMENT_SHADER_ENTRY: &str = "fs_main";

#[derive(Debug)]
pub struct AtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline_params: PipelineParams,
    fs_bind_group_layout: BindGroupLayout,
}

impl AtlasRenderer {
    pub fn new<VertexT: VertexLayout>(
        device: &wgpu::Device,
        atlas_format: wgpu::TextureFormat,
        shader: wgpu::ShaderModuleDescriptor<'_>,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = device.create_shader_module(shader);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atlas Pipeline Layout"),
            bind_group_layouts: &[&fs_bind_group_layout],
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

        Some(RenderBatch {
            fs_bind_group: Some(fs_bind_group),
            vertex_buffer,
            count: instances.len(),
        })
    }
}

pub trait AtlasInstance {
    type Vertex: Pod;

    fn to_vertices(&self) -> [Self::Vertex; 4];
}

#[derive(Debug, Deref)]
pub struct BindGroupLayout(wgpu::BindGroupLayout);

impl BindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = BindGroupLayoutBuilder::fragment_stage()
            .texture()
            .sampler()
            .build("Color Atlas Bind Group Layout", device);

        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
        texture_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Color Atlas Bind Group"),
            layout: &self.0,
            entries: bind_group_entries!(0 => texture_view, 1 => texture_sampler),
        })
    }
}
