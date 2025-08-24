use bytemuck::{Pod, Zeroable};
use massive_geometry::{Color, Rect};
use massive_shapes::Shape;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use crate::{
    pods::{self, AsBytes, ToPod, VertexLayout},
    renderer::{RenderContext, RenderVisual},
    scene::LocationMatrices,
    tools::{QuadIndexBuffer, create_pipeline},
};

const FRAGMENT_SHADER_ENTRY: &str = "fs_main";

/// Shape selector values shared with `shape_renderer.wgsl`.
///
/// Filled variants are in the 0 range; non-filled (stroked) start at 10.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u32)]
pub enum ShapeSelector {
    Rect = 0,
    RoundedRect = 1,
    Circle = 2,
    Ellipse = 3,
    ChamferRect = 4,
    // Non-filled
    StrokeRect = 10,
}

impl From<ShapeSelector> for u32 {
    fn from(value: ShapeSelector) -> Self {
        value as u32
    }
}

#[derive(Debug)]
pub struct ShapeRenderer {
    pipeline: wgpu::RenderPipeline,
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

        Self { pipeline }
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub fn render<'a>(
        &self,
        visual_matrices: &LocationMatrices,
        visuals: impl Iterator<Item = &'a RenderVisual>,
        render_context: &mut RenderContext,
    ) {
        let pass = &mut render_context.pass;

        pass.set_pipeline(self.pipeline());
        for visual in visuals {
            if let Some(ref shape_batch) = visual.batches.shapes {
                let model_matrix =
                    *render_context.pixel_matrix * visual_matrices.get(visual.location_id);
                let vm = render_context.view_projection_matrix * model_matrix;
                pass.set_push_constants(wgpu::ShaderStages::VERTEX, 0, vm.to_pod().as_bytes());
                pass.set_vertex_buffer(0, shape_batch.vertex_buffer.slice(..));
                pass.draw_indexed(
                    0..(shape_batch.count * QuadIndexBuffer::INDICES_PER_QUAD) as u32,
                    0,
                    0..1,
                );
            }
        }
    }

    /// Build a batch directly from a slice of `massive_shapes::Shape` objects.
    /// Ignores glyph runs (text); only geometric shapes are converted.
    pub fn batch_from_shapes(
        &self,
        device: &wgpu::Device,
        shapes: &[massive_shapes::Shape],
    ) -> Option<Batch> {
        let mut vertices: Vec<Vertex> = Vec::with_capacity(shapes.len() * 4); // upper bound
        let mut quad_count = 0usize;
        const B: f32 = 1.0; // 1px AA fringe in model space

        // Helper that emits a single expanded quad with AA fringe and normalized tex coords.
        let mut emit = |rect: &Rect, selector: ShapeSelector, data: (f32, f32), color: Color| {
            let left = rect.left as f32;
            let top = rect.top as f32;
            let right = rect.right as f32;
            let bottom = rect.bottom as f32;
            let w = right - left;
            let h = bottom - top;
            let size = (w, h);
            let selector: u32 = selector.into();
            vertices.extend([
                Vertex::new(
                    (left - B, top - B, 0.0),
                    (-B, -B),
                    selector,
                    size,
                    data,
                    color,
                ),
                Vertex::new(
                    (left - B, bottom + B, 0.0),
                    (-B, h + B),
                    selector,
                    size,
                    data,
                    color,
                ),
                Vertex::new(
                    (right + B, bottom + B, 0.0),
                    (w + B, h + B),
                    selector,
                    size,
                    data,
                    color,
                ),
                Vertex::new(
                    (right + B, top - B, 0.0),
                    (w + B, -B),
                    selector,
                    size,
                    data,
                    color,
                ),
            ]);
            quad_count += 1;
        };

        for shape in shapes.iter() {
            match shape {
                Shape::GlyphRun(_) => {}
                Shape::Rect(r) => emit(&r.rect, ShapeSelector::Rect, (0.0, 0.0), r.color),
                Shape::RoundRect(r) => emit(
                    &r.rect,
                    ShapeSelector::RoundedRect,
                    (r.corner_radius, 0.0),
                    r.color,
                ),
                Shape::ChamferRect(r) => emit(
                    &r.rect,
                    ShapeSelector::ChamferRect,
                    (r.chamfer, 0.0),
                    r.color,
                ),
                Shape::Circle(c) => emit(&c.rect, ShapeSelector::Circle, (0.0, 0.0), c.color),
                Shape::Ellipse(e) => emit(&e.rect, ShapeSelector::Ellipse, (0.0, 0.0), e.color),
                Shape::StrokeRect(s) => emit(
                    &s.rect,
                    ShapeSelector::StrokeRect,
                    (s.stroke.width as f32, s.stroke.height as f32),
                    s.color,
                ),
            }
        }

        if quad_count == 0 {
            return None;
        }

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Shape Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(Batch {
            vertex_buffer,
            quad_count,
        })
    }
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

#[derive(Debug)]
pub struct Batch {
    pub vertex_buffer: wgpu::Buffer,
    pub quad_count: usize,
}
