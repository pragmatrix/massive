use std::rc::Rc;

use crate::geometry::Matrix4;

#[derive(Debug)]
pub enum Shape {
    Glyph(Glyph),
}

#[derive(Debug)]
pub struct Glyph {
    pub matrix: Rc<Matrix4>,
    pub position: GlyphPosition,
}

#[derive(Debug)]
pub struct GlyphPosition {
    // This is for rendering the image of the glyph.
    pub cache_key: cosmic_text::CacheKey,
    pub hitbox_pos: (i32, i32),
    pub hitbox_width: f32,
}
