use bytemuck::Pod;
use derive_more::Deref;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::{
    bind_group_entries,
    glyph::GlyphAtlas,
    pods::{self, AsBytes, VertexLayout},
    renderer::PreparationContext,
    text_layer::QuadBatch,
    tools::{BindGroupLayoutBuilder, create_pipeline, texture_sampler},
};

const FRAGMENT_SHADER_ENTRY: &str = "fs_main";

pub struct AtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    fs_bind_group_layout: BindGroupLayout,
}

impl AtlasRenderer {
    pub fn new<InstanceVertexT: VertexLayout>(
        device: &wgpu::Device,
        atlas_format: wgpu::TextureFormat,
        shader: wgpu::ShaderModuleDescriptor<'_>,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let fs_bind_group_layout = BindGroupLayout::new(device);

        let shader = &device.create_shader_module(shader);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atlas Pipeline Layout"),
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

        // One vertex buffer layout describing instance attributes.
        let vertex_layout = [InstanceVertexT::layout()];

        let pipeline = create_pipeline(
            "Atlas Pipeline",
            device,
            shader,
            FRAGMENT_SHADER_ENTRY,
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            atlas: GlyphAtlas::new(device, atlas_format),
            texture_sampler: texture_sampler::linear_clamping(device),
            fs_bind_group_layout,
            pipeline,
        }
    }

    // Build a batch from instance data, uploading one instance per glyph.
    pub fn batch_instances<InstanceVertexT: Pod>(
        &self,
        context: &PreparationContext,
        instances: &[InstanceVertexT],
    ) -> Option<QuadBatch> {
        if instances.is_empty() {
            return None;
        }

        let device = context.device;

        let fs_bind_group = self.fs_bind_group_layout.create_bind_group(
            device,
            self.atlas.texture_view(),
            &self.texture_sampler,
        );

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Atlas Instance Buffer"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(QuadBatch {
            fs_bind_group,
            vertex_buffer,
            quad_count: instances.len(),
        })
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }
}

// Instance vertex types used by the shaders
pub mod sdf_atlas {
    use bytemuck::{Pod, Zeroable};

    #[repr(C)]
    #[derive(Copy, Clone, Debug, Pod, Zeroable)]
    pub struct InstanceVertex {
        // pos_lt.xy, pos_rb.xy
        pub pos_lt: [f32; 2],
        pub pos_rb: [f32; 2],
        // uv_lt.xy, uv_rb.xy
        pub uv_lt: [f32; 2],
        pub uv_rb: [f32; 2],
        // color rgba
        pub color: [f32; 4],
    }

    impl super::VertexLayout for InstanceVertex {
        fn layout() -> wgpu::VertexBufferLayout<'static> {
            use std::mem::size_of;
            use wgpu::{BufferAddress, VertexAttribute, VertexBufferLayout, VertexStepMode};
            const ATTRS: [VertexAttribute; 5] = wgpu::vertex_attr_array![
                0 => Float32x2, // pos_lt
                1 => Float32x2, // pos_rb
                2 => Float32x2, // uv_lt
                3 => Float32x2, // uv_rb
                4 => Float32x4  // color
            ];
            VertexBufferLayout {
                array_stride: size_of::<InstanceVertex>() as BufferAddress,
                step_mode: VertexStepMode::Instance,
                attributes: &ATTRS,
            }
        }
    }
}

pub mod color_atlas {
    use bytemuck::{Pod, Zeroable};

    #[repr(C)]
    #[derive(Copy, Clone, Debug, Pod, Zeroable)]
    pub struct InstanceVertex {
        pub pos_lt: [f32; 2],
        pub pos_rb: [f32; 2],
        pub uv_lt: [f32; 2],
        pub uv_rb: [f32; 2],
    }

    impl super::VertexLayout for InstanceVertex {
        fn layout() -> wgpu::VertexBufferLayout<'static> {
            use std::mem::size_of;
            use wgpu::{BufferAddress, VertexAttribute, VertexBufferLayout, VertexStepMode};
            const ATTRS: [VertexAttribute; 4] = wgpu::vertex_attr_array![
                0 => Float32x2, // pos_lt
                1 => Float32x2, // pos_rb
                2 => Float32x2, // uv_lt
                3 => Float32x2  // uv_rb
            ];
            VertexBufferLayout {
                array_stride: size_of::<InstanceVertex>() as BufferAddress,
                step_mode: VertexStepMode::Instance,
                attributes: &ATTRS,
            }
        }
    }
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
