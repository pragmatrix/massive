use bytemuck::{Pod, Zeroable};
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

    pub fn render<'a>(
        &self,
        visuals: impl Iterator<Item = &'a RenderVisual>,
        visual_matrices: &LocationMatrices,
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
        use massive_geometry::Point; // For convenient point conversions
        use massive_shapes::Shape;

        let mut vertices: Vec<Vertex> = Vec::with_capacity(shapes.len() * 4); // upper bound (glyphs skipped later)

        let mut quad_count = 0usize;
        const B: f32 = 1.0; // 1px AA border in model coordinates

        for shape in shapes.iter() {
            match shape {
                Shape::GlyphRun(_) => {}
                Shape::Rect(r) => {
                    let w = (r.rect.right - r.rect.left) as f32;
                    let h = (r.rect.bottom - r.rect.top) as f32;
                    let selector = ShapeSelector::Rect as u32;
                    let size = (w, h);
                    let data = (0.0, 0.0);
                    let color = r.color;
                    let lt: Point = (r.rect.left, r.rect.top).into();
                    let rb: Point = (r.rect.right, r.rect.bottom).into();
                    let lb: Point = (r.rect.left, r.rect.bottom).into();
                    let rt: Point = (r.rect.right, r.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
                Shape::RoundRect(r) => {
                    let w = (r.rect.right - r.rect.left) as f32;
                    let h = (r.rect.bottom - r.rect.top) as f32;
                    let selector = ShapeSelector::RoundedRect as u32;
                    let size = (w, h);
                    let data = (r.corner_radius, 0.0);
                    let color = r.color;
                    let lt: Point = (r.rect.left, r.rect.top).into();
                    let rb: Point = (r.rect.right, r.rect.bottom).into();
                    let lb: Point = (r.rect.left, r.rect.bottom).into();
                    let rt: Point = (r.rect.right, r.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
                Shape::ChamferRect(r) => {
                    let w = (r.rect.right - r.rect.left) as f32;
                    let h = (r.rect.bottom - r.rect.top) as f32;
                    let selector = ShapeSelector::ChamferRect as u32;
                    let size = (w, h);
                    let data = (r.chamfer, 0.0);
                    let color = r.color;
                    let lt: Point = (r.rect.left, r.rect.top).into();
                    let rb: Point = (r.rect.right, r.rect.bottom).into();
                    let lb: Point = (r.rect.left, r.rect.bottom).into();
                    let rt: Point = (r.rect.right, r.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
                Shape::Circle(c) => {
                    let w = (c.rect.right - c.rect.left) as f32;
                    let h = (c.rect.bottom - c.rect.top) as f32;
                    let selector = ShapeSelector::Circle as u32;
                    let size = (w, h);
                    let data = (0.0, 0.0);
                    let color = c.color;
                    let lt: Point = (c.rect.left, c.rect.top).into();
                    let rb: Point = (c.rect.right, c.rect.bottom).into();
                    let lb: Point = (c.rect.left, c.rect.bottom).into();
                    let rt: Point = (c.rect.right, c.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
                Shape::Ellipse(e) => {
                    let w = (e.rect.right - e.rect.left) as f32;
                    let h = (e.rect.bottom - e.rect.top) as f32;
                    let selector = ShapeSelector::Ellipse as u32;
                    let size = (w, h);
                    let data = (0.0, 0.0);
                    let color = e.color;
                    let lt: Point = (e.rect.left, e.rect.top).into();
                    let rb: Point = (e.rect.right, e.rect.bottom).into();
                    let lb: Point = (e.rect.left, e.rect.bottom).into();
                    let rt: Point = (e.rect.right, e.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
                Shape::StrokeRect(s) => {
                    let w = (s.rect.right - s.rect.left) as f32;
                    let h = (s.rect.bottom - s.rect.top) as f32;
                    let selector = ShapeSelector::StrokeRect as u32;
                    let size = (w, h);
                    let data = (s.stroke.width as f32, s.stroke.height as f32);
                    let color = s.color;
                    let lt: Point = (s.rect.left, s.rect.top).into();
                    let rb: Point = (s.rect.right, s.rect.bottom).into();
                    let lb: Point = (s.rect.left, s.rect.bottom).into();
                    let rt: Point = (s.rect.right, s.rect.top).into();
                    vertices.extend([
                        Vertex::new(
                            ((lt.x as f32) - B, (lt.y as f32) - B, 0.0),
                            (-B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((lb.x as f32) - B, (lb.y as f32) + B, 0.0),
                            (-B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rb.x as f32) + B, (rb.y as f32) + B, 0.0),
                            (w + B, h + B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                        Vertex::new(
                            ((rt.x as f32) + B, (rt.y as f32) - B, 0.0),
                            (w + B, -B),
                            selector,
                            size,
                            data,
                            color,
                        ),
                    ]);
                    quad_count += 1;
                }
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

#[derive(Debug)]
pub struct Batch {
    pub vertex_buffer: wgpu::Buffer,
    pub quad_count: usize,
}
