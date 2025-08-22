use bytemuck::{Pod, Zeroable};
use derive_more::Deref;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::{
    bind_group_entries,
    pods::{self, AsBytes, VertexLayout},
    tools::{BindGroupLayoutBuilder, QuadIndexBuffer, create_pipeline},
};

const FRAGMENT_SHADER_ENTRY: &str = "fs_main";

#[derive(Debug)]
pub struct ShapeRenderer {
    pipeline: wgpu::RenderPipeline,
    index_buffer: QuadIndexBuffer,
}

impl ShapeRenderer {
    /// Create a new shape renderer. The vertex layout must match `shape/shape.wgsl`.
    pub fn new<VertexT: VertexLayout>(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = &device.create_shader_module(wgpu::include_wgsl!("shape_renderer.wgsl"));

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shape Pipeline Layout"),
            bind_group_layouts: &[],
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

        // Triangle list pipeline (will use quad index buffer like text layer)
        let pipeline = create_pipeline(
            "Shape Pipeline",
            device,
            shader,
            FRAGMENT_SHADER_ENTRY,
            &vertex_layout,
            &pipeline_layout,
            &targets,
        );

        Self {
            pipeline,
            index_buffer: QuadIndexBuffer::new(device),
        }
    }

    /// Build a batch from a slice of instances sharing the same model_view matrix.
    pub fn batch<InstanceT: ShapeInstance>(
        &self,
        device: &wgpu::Device,
        instances: &[InstanceT],
    ) -> Option<Batch> {
        if instances.is_empty() {
            return None;
        }

        let mut vertices = Vec::with_capacity(instances.len() * 4);
        for instance in instances {
            vertices.extend(instance.to_vertices());
        }

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Shape Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(Batch {
            vertex_buffer,
            quad_count: instances.len(),
        })
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub fn ensure_index_capacity(&mut self, device: &wgpu::Device, quads: usize) {
        self.index_buffer.ensure_can_index_num_quads(device, quads);
    }

    pub fn set_index_buffer<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, max_quads: usize) {
        self.index_buffer.set(pass, max_quads);
    }
}

pub trait ShapeInstance {
    type Vertex: Pod;

    fn to_vertices(&self) -> [Self::Vertex; 4];
}

/// Vertex format for `shape/shape.wgsl`.
/// locations: 0=position, 1=unorm_tex_coords, 2=shape_selector, 3=shape_size, 4=shape_data, 5=color
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: pods::Vertex,
    pub unorm_tex_coords: [f32; 2],
    pub shape_selector: u32,
    pub shape_size: [f32; 2],
    pub shape_data: [f32; 2],
    pub color: pods::Color,
}

impl Vertex {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        position: impl Into<pods::Vertex>,
        unorm_tex_coords: (f32, f32),
        shape_selector: u32,
        shape_size: (f32, f32),
        shape_data: (f32, f32),
        color: impl Into<pods::Color>,
    ) -> Self {
        Self {
            position: position.into(),
            unorm_tex_coords: [unorm_tex_coords.0, unorm_tex_coords.1],
            shape_selector,
            shape_size: [shape_size.0, shape_size.1],
            shape_data: [shape_data.0, shape_data.1],
            color: color.into(),
        }
    }
}

impl VertexLayout for Vertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        // Order must match struct field order. Shader location indices map accordingly.
        const ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
            0 => Float32x3, // position
            1 => Float32x2, // unorm_tex_coords
            2 => Uint32,    // shape_selector
            3 => Float32x2, // shape_size
            4 => Float32x2, // shape_data
            5 => Float32x4  // color
        ];

        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

#[derive(Debug, Deref)]
pub struct VsBindGroupLayout(wgpu::BindGroupLayout);

impl VsBindGroupLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = BindGroupLayoutBuilder::vertex_stage()
            .uniform()
            .build("Shape VS Bind Group Layout", device);
        Self(layout)
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        model_view_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shape VS Bind Group"),
            layout: &self.0,
            entries: bind_group_entries!(0 => model_view_buffer),
        })
    }
}

#[derive(Debug)]
pub struct Batch {
    pub vertex_buffer: wgpu::Buffer,
    pub quad_count: usize,
}
