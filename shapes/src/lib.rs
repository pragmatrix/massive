mod text;

pub use text::*;

use derive_more::From;

use massive_geometry::{self as geometry, Color, Size};

#[derive(Debug, Clone, From)]
pub enum Shape {
    Rect(Rect),
    RoundRect(RoundRect),
    Circle(Circle),
    StrokeRect(StrokeRect),
    GlyphRun(GlyphRun),
}

#[derive(Debug, Clone)]
pub struct Rect {
    pub rect: geometry::Rect,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct RoundRect {
    pub rect: geometry::Rect,
    pub corner_radius: f32,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct Circle {
    pub rect: geometry::Rect,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct StrokeRect {
    pub rect: geometry::Rect,
    pub stroke: Size,
    pub color: Color,
}
