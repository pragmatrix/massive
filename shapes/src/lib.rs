mod text;

pub use text::*;

use std::{any::Any, fmt, mem, ops};

use derive_more::From;
use massive_geometry::{self as geometry, Color, Size};
use smallbox::{SmallBox, smallbox};

#[derive(Debug, Clone, From, PartialEq)]
pub enum Shape {
    Rect(Rect),
    RoundRect(RoundRect),
    Circle(Circle),
    Ellipse(Ellipse),
    BeveledRect(BeveledRect),
    StrokeRect(StrokeRect),
    GlyphRun(GlyphRun),
    Custom(Custom),
}

const CUSTOM_EMBEDDED_SIZE: usize = 7;

const _: () = {
    // GlyphRun is expected to be the biggest contender. If that changes, we want to know.
    // Also it seems that the enum discriminant is stored inside the space of the GlyphRun.
    assert!(mem::size_of::<GlyphRun>() == mem::size_of::<Shape>());
    // It seems that there are three words overhead, so we keep that as a constraint.
    assert!(mem::size_of::<Shape>() == (CUSTOM_EMBEDDED_SIZE + 3) * mem::size_of::<usize>());
    // Niche optimization possible for Shape?
    assert!(mem::size_of::<Option<Shape>>() == mem::size_of::<Shape>());
    // Niche optimization possible for Custom?
    assert!(mem::size_of::<Option<Custom>>() == mem::size_of::<Custom>());
};

impl Shape {
    // Construct a custom shape from any suitable type
    pub fn custom<S: CustomShape>(shape: S) -> Self {
        Self::Custom(Custom(smallbox!(shape)))
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

/// A rectangle with chamfered (beveled) corners.
///
/// The `corner_mask` field controls which corners are beveled using a 4-bit mask:
/// - bit 0 (0x1): top-left corner
/// - bit 1 (0x2): top-right corner
/// - bit 2 (0x4): bottom-right corner
/// - bit 3 (0x8): bottom-left corner
///
/// By default, all corners are beveled (corner_mask = 0b1111).
#[derive(Debug, Clone, PartialEq)]
pub struct BeveledRect {
    pub rect: geometry::Rect,
    pub chamfer: f32,
    /// Bitmask controlling which corners are beveled (clockwise from top-left).
    /// Default: 0b1111 (all corners beveled).
    pub corner_mask: u8,
    pub color: Color,
}

impl BeveledRect {
    pub fn new(rect: impl Into<geometry::Rect>, chamfer: f32, color: impl Into<Color>) -> Self {
        Self {
            rect: rect.into(),
            chamfer,
            corner_mask: 0b1111, // All corners beveled by default
            color: color.into(),
        }
    }

    /// Enable or disable beveling on the top-left corner (bit 0).
    pub fn with_top_left(self, enabled: bool) -> Self {
        self.set_corner_bit(0, enabled)
    }

    /// Enable or disable beveling on the top-right corner (bit 1).
    pub fn with_top_right(self, enabled: bool) -> Self {
        self.set_corner_bit(1, enabled)
    }

    /// Enable or disable beveling on the bottom-right corner (bit 2).
    pub fn with_bottom_right(self, enabled: bool) -> Self {
        self.set_corner_bit(2, enabled)
    }

    /// Enable or disable beveling on the bottom-left corner (bit 3).
    pub fn with_bottom_left(self, enabled: bool) -> Self {
        self.set_corner_bit(3, enabled)
    }

    /// Set the corner bit at the given index (0-3).
    fn set_corner_bit(mut self, bit_index: u8, enabled: bool) -> Self {
        debug_assert!(bit_index < 4, "Corner bit index must be 0-3");
        if enabled {
            self.corner_mask |= 1 << bit_index;
        } else {
            self.corner_mask &= !(1 << bit_index);
        }
        debug_assert!(self.corner_mask <= 0x0F, "Corner mask must be 4 bits");
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
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

type CustomSmallBox = SmallBox<dyn CustomShape, [usize; CUSTOM_EMBEDDED_SIZE]>;

#[derive(Debug, PartialEq)]
pub struct Custom(CustomSmallBox);

impl ops::Deref for Custom {
    type Target = dyn CustomShape;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl Clone for Custom {
    fn clone(&self) -> Self {
        Self(self.0.clone_smallbox())
    }
}

// Supports cloning of boxed custom shapes. Send + Sync so shapes can be shared/moved across threads.
pub trait CustomShape: fmt::Debug + Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn clone_smallbox(&self) -> CustomSmallBox;
    fn eq_dyn(&self, other: &dyn CustomShape) -> bool;
}

// Blanket impl now requires Clone (for Shape: Clone) plus Send + Sync to satisfy the supertraits.
impl<T: fmt::Debug + Any + Clone + PartialEq + Send + Sync> CustomShape for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_smallbox(&self) -> CustomSmallBox {
        smallbox!(self.clone())
    }

    fn eq_dyn(&self, other: &dyn CustomShape) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
}

// Provide PartialEq for trait objects of CustomShape using the trait's own `eq_dyn` method.
impl PartialEq for dyn CustomShape {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other)
    }
}
