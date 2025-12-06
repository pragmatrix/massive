use bytemuck::{Pod, Zeroable};
use massive_geometry::{Color, Vector3};

use super::atlas_renderer::AtlasInstance;
use crate::{
    glyph::glyph_atlas,
    pods::{self, VertexLayout},
};

#[derive(Debug)]
pub struct Instance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Vector3; 4],
    pub color: Color,
}

impl AtlasInstance for Instance {
    type Vertex = Vertex;

    fn to_vertices(&self) -> [Self::Vertex; 4] {
        let r = self.atlas_rect;
        // ADR: u/v normalization is done in the shader, because its probably free and we can
        // reuse vertices when the texture atlas grows.
        let (ltx, lty) = (r.min.x as f32, r.min.y as f32);
        let (rbx, rby) = (r.max.x as f32, r.max.y as f32);

        let v = &self.vertices;
        let color = self.color;
        [
            Vertex::new(v[0], (ltx, lty), color),
            Vertex::new(v[1], (ltx, rby), color),
            Vertex::new(v[2], (rbx, rby), color),
            Vertex::new(v[3], (rbx, lty), color),
        ]
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: pods::Vertex,
    pub tex_coords: [f32; 2],
    /// OO: Use one byte per color component?
    pub color: pods::Color,
}

impl Vertex {
    pub fn new(
        position: impl Into<pods::Vertex>,
        uv: (f32, f32),
        color: impl Into<pods::Color>,
    ) -> Self {
        Self {
            position: position.into(),
            tex_coords: [uv.0, uv.1],
            color: color.into(),
        }
    }
}

impl VertexLayout for Vertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];

        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}
