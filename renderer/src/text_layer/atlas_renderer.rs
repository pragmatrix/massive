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

#[derive(Debug)]
pub struct AtlasRenderer {
    pub atlas: GlyphAtlas,
    texture_sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
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

        let vertex_layout = [VertexT::layout()];

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

    // Convert a number of instances to a batch.
    pub fn batch<InstanceT: AtlasInstance>(
        &self,
        context: &PreparationContext,
        instances: &[InstanceT],
    ) -> Option<QuadBatch> {
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

pub trait AtlasInstance {
    type Vertex: Pod;

    fn to_vertices(&self) -> [Self::Vertex; 4];
}

pub mod sdf_atlas {
    use massive_geometry::{Color, Point3};

    use super::AtlasInstance;
    use crate::{glyph::glyph_atlas, pods::TextureColorVertex};

    #[derive(Debug)]
    pub struct QuadInstance {
        pub atlas_rect: glyph_atlas::Rectangle,
        pub vertices: [Point3; 4],
        pub color: Color,
    }

    impl AtlasInstance for QuadInstance {
        type Vertex = TextureColorVertex;

        fn to_vertices(&self) -> [Self::Vertex; 4] {
            let r = self.atlas_rect;
            // ADR: u/v normalization is done in the shader, because its probably free and we don't
            // have to care about the atlas texture growing as long the rects stay the same.
            let (ltx, lty) = (r.min.x as f32, r.min.y as f32);
            let (rbx, rby) = (r.max.x as f32, r.max.y as f32);

            let v = &self.vertices;
            let color = self.color;
            [
                TextureColorVertex::new(v[0], (ltx, lty), color),
                TextureColorVertex::new(v[1], (ltx, rby), color),
                TextureColorVertex::new(v[2], (rbx, rby), color),
                TextureColorVertex::new(v[3], (rbx, lty), color),
            ]
        }
    }
}

pub mod color_atlas {
    use massive_geometry::Point3;

    use super::AtlasInstance;
    use crate::{glyph::glyph_atlas, pods::TextureVertex};

    #[derive(Debug)]
    pub struct QuadInstance {
        pub atlas_rect: glyph_atlas::Rectangle,
        pub vertices: [Point3; 4],
    }

    impl AtlasInstance for QuadInstance {
        type Vertex = TextureVertex;

        fn to_vertices(&self) -> [Self::Vertex; 4] {
            let r = self.atlas_rect;
            // ADR: u/v normalization is done in the shader. Its probably free, and we don't have to
            // care about the atlas texture growing as long the rects stay the same.
            let (ltx, lty) = (r.min.x as f32, r.min.y as f32);
            let (rbx, rby) = (r.max.x as f32, r.max.y as f32);
            let v = &self.vertices;
            [
                TextureVertex::new(v[0], (ltx, lty)),
                TextureVertex::new(v[1], (ltx, rby)),
                TextureVertex::new(v[2], (rbx, rby)),
                TextureVertex::new(v[3], (rbx, lty)),
            ]
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
