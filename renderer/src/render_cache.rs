//! The render cache. This cache pre-processes shapes, creates their textures, and caches them.

use cosmic_text::FontSystem;
use granularity_shapes::{Glyph, Shape};

struct RenderCache {
    font_system: FontSystem,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
        }
    }

    pub fn prepare_frame(&mut self, shapes: &[Shape]) {
        for shape in shapes {
            self.prepare_shape(shape);
        }
    }

    pub fn prepare_shape(&mut self, shape: &Shape) {
        match shape {
            Shape::Glyph(glyph) => self.prepare_glyph(&glyph),
        }
    }

    pub fn prepare_glyph(&mut self, glyph: &Glyph) {



    }
}
