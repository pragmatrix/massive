use massive_geometry::Vector3;

use super::atlas_renderer::AtlasInstance;
use crate::glyph::glyph_atlas;
use crate::pods::TextureVertex;

#[derive(Debug)]
pub struct Instance {
    pub atlas_rect: glyph_atlas::Rectangle,
    pub vertices: [Vector3; 4],
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
