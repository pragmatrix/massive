use bytemuck::{Pod, Zeroable};
use massive_geometry::Point3;

use super::atlas_renderer::AtlasInstance;
use crate::{
    glyph::glyph_atlas,
    pods::{Vertex, VertexLayout},
};

#[derive(Debug)]
pub struct Instance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Point3; 4],
}

impl AtlasInstance for Instance {
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

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TextureVertex {
    pub position: Vertex,
    pub tex_coords: [f32; 2],
}

impl TextureVertex {
    #[allow(unused)]
    pub fn new(position: impl Into<Vertex>, uv: (f32, f32)) -> Self {
        Self {
            position: position.into(),
            tex_coords: [uv.0, uv.1],
        }
    }
}

impl VertexLayout for TextureVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];

        wgpu::VertexBufferLayout {
            array_stride: size_of::<TextureVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}
