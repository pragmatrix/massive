mod text;

pub use text::*;

use derive_more::From;

use massive_geometry::{self as geometry, Color, Size};

#[derive(Debug, Clone, From)]
pub enum Shape {
    Rect(Rect),
    RoundRect(RoundRect),
    Circle(Circle),
    Ellipse(Ellipse),
    ChamferRect(ChamferRect),
    StrokeRect(StrokeRect),
    GlyphRun(GlyphRun),
}

#[derive(Debug, Clone)]
pub struct Rect {
    pub rect: geometry::Rect,
    pub color: Color,
}

impl Rect {
    pub fn new(rect: impl Into<geometry::Rect>, color: impl Into<Color>) -> Self {
        Self {
            rect: rect.into(),
            color: color.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoundRect {
    pub rect: geometry::Rect,
    pub corner_radius: f32,
    pub color: Color,
}

impl RoundRect {
    pub fn new(
        rect: impl Into<geometry::Rect>,
        corner_radius: f32,
        color: impl Into<Color>,
    ) -> Self {
        Self {
            rect: rect.into(),
            corner_radius,
            color: color.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Circle {
    pub rect: geometry::Rect,
    pub color: Color,
}

impl Circle {
    pub fn new(rect: impl Into<geometry::Rect>, color: impl Into<Color>) -> Self {
        Self {
            rect: rect.into(),
            color: color.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ellipse {
    pub rect: geometry::Rect,
    pub color: Color,
}

impl Ellipse {
    pub fn new(rect: impl Into<geometry::Rect>, color: impl Into<Color>) -> Self {
        Self {
            rect: rect.into(),
            color: color.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChamferRect {
    pub rect: geometry::Rect,
    pub chamfer: f32,
    pub color: Color,
}

impl ChamferRect {
    pub fn new(rect: impl Into<geometry::Rect>, chamfer: f32, color: impl Into<Color>) -> Self {
        Self {
            rect: rect.into(),
            chamfer,
            color: color.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrokeRect {
    pub rect: geometry::Rect,
    pub stroke: Size,
    pub color: Color,
}

impl StrokeRect {
    pub fn new(
        rect: impl Into<geometry::Rect>,
        stroke: impl Into<Size>,
        color: impl Into<Color>,
    ) -> Self {
        Self {
            rect: rect.into(),
            stroke: stroke.into(),
            color: color.into(),
        }
    }
}
