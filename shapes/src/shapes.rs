use std::rc::Rc;

use cosmic_text as text;
use massive_geometry::{Color, Vector3};

use crate::geometry::{Bounds, Matrix4};

#[derive(Debug)]
pub enum Shape {
    /// This shape describes a number of glyphs that should be rendered at
    GlyphRun {
        // Model transformation
        model_matrix: Rc<Matrix4>,
        // Local translation of the glyph runs.
        //
        // This is separated from the view transformation matrix to support instancing of glyphs.
        // TODO: May put this into [`GlyphRun`]
        translation: Vector3,
        run: GlyphRun,
    },
}

#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub metrics: GlyphRunMetrics,
    pub text_color: Color,
    pub glyphs: Vec<PositionedGlyph>,
}

impl GlyphRun {
    pub fn new(metrics: GlyphRunMetrics, text_color: Color, glyphs: Vec<PositionedGlyph>) -> Self {
        Self {
            metrics,
            text_color,
            glyphs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphRunMetrics {
    pub max_ascent: u32,
    pub max_descent: u32,
    pub width: u32,
}

impl GlyphRunMetrics {
    /// Size of the glyph run in font-size pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.max_ascent + self.max_descent)
    }
}

/// A glyph positioned inside a [`GlyphRun`].
#[derive(Debug, Clone)]
pub struct PositionedGlyph {
    // This is for rendering the image of the glyph.
    pub key: text::CacheKey,
    pub hitbox_pos: (i32, i32),
    pub hitbox_width: f32,
}

impl PositionedGlyph {
    pub fn new(key: text::CacheKey, hitbox_pos: (i32, i32), hitbox_width: f32) -> Self {
        Self {
            key,
            hitbox_pos,
            hitbox_width,
        }
    }

    // The bounds enclosing a pixel at the offset of the hitbox
    pub fn pixel_bounds_at(&self, offset: (u32, u32)) -> Bounds {
        let x = self.hitbox_pos.0 + offset.0 as i32;
        let y = self.hitbox_pos.1 + offset.1 as i32;

        Bounds::new((x as f64, y as f64), ((x + 1) as f64, (y + 1) as f64))
    }
}
