mod text;

pub use text::*;

use derive_more::From;
use massive_geometry::{self as geometry, Color, Size};
use std::{any::Any, fmt};

#[derive(Debug, Clone, From)]
pub enum Shape {
    Rect(Rect),
    RoundRect(RoundRect),
    Circle(Circle),
    Ellipse(Ellipse),
    ChamferRect(ChamferRect),
    StrokeRect(StrokeRect),
    GlyphRun(GlyphRun),
    Custom(Box<dyn CustomShape>),
}

impl Shape {
    // Construct a custom shape from any suitable type
    pub fn custom<S: CustomShape>(shape: S) -> Self {
        Self::Custom(Box::new(shape))
    }

    // Attempt to downcast a custom shape to a concrete type
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        match self {
            Shape::Custom(c) => c.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }

    // Helper to check if shape is a custom type of T
    pub fn is<T: 'static>(&self) -> bool {
        self.downcast_ref::<T>().is_some()
    }
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

// Supports cloning of boxed custom shapes. Send + Sync so shapes can be shared/moved across threads.
pub trait CustomShape: fmt::Debug + Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn CustomShape>;
}

// Blanket impl now requires Clone (for Shape: Clone) plus Send + Sync to satisfy the supertraits.
impl<T: fmt::Debug + Any + Clone + Send + Sync> CustomShape for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn clone_box(&self) -> Box<dyn CustomShape> {
        Box::new(self.clone())
    }
}

// Enable cloning Box<dyn CustomShape>
impl Clone for Box<dyn CustomShape> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
