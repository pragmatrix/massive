//! Taken from skia-safe at 20230701
use std::ops::{Add, Sub};

use crate::{Centered, Contains, Point, Size, Vector};

/// A basic rectangle representation. Meant to be sorted and with finite values only.
// Architecture: Think about replacing this with an euclid Rect / Box
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    #[must_use]
    pub fn new(origin: impl Into<Point>, size: impl Into<Size>) -> Self {
        (origin.into(), size.into()).into()
    }

    #[must_use]
    pub fn from_size(size: impl Into<Size>) -> Self {
        let size = size.into();
        (Point::default(), size).into()
    }

    pub fn is_empty(&self) -> bool {
        // We write it as the NOT of a non-empty rect, so we will return true if any values
        // are NaN.
        !(self.left < self.right && self.top < self.bottom)
    }

    pub fn is_sorted(&self) -> bool {
        self.left <= self.right && self.top <= self.bottom
    }

    // TODO: Guarantee that Rects are always finite.
    pub fn is_finite(&self) -> bool {
        let mut accum: f64 = 0.0;
        accum *= self.left;
        accum *= self.top;
        accum *= self.right;
        accum *= self.bottom;

        // accum is either NaN or it is finite (zero).
        debug_assert!(0.0 == accum || accum.is_nan());

        // value==value will be true iff value is not NaN
        // TODO: is it faster to say !accum or accum==accum?
        !accum.is_nan()
    }

    pub fn size(&self) -> Size {
        (self.right - self.left, self.bottom - self.top).into()
    }

    pub fn origin(&self) -> Point {
        (self.left, self.top).into()
    }

    pub fn center(&self) -> Point {
        // don't use (fLeft + fBottom) * 0.5 as that might overflow before the 0.5
        (
            self.left * 0.5 + self.right * 0.5,
            self.top * 0.5 + self.bottom * 0.5,
        )
            .into()
    }

    pub fn end(&self) -> Point {
        (self.right, self.bottom).into()
    }

    /// Returns a clockwise quad starting a left / top.
    pub fn to_quad(&self) -> [Point; 4] {
        [
            (self.left, self.top).into(),
            (self.right, self.top).into(),
            (self.right, self.bottom).into(),
            (self.left, self.bottom).into(),
        ]
    }

    #[must_use]
    pub fn with_inset(&self, d: impl Into<Vector>) -> Self {
        let d = d.into();
        (
            self.left + d.x,
            self.top + d.y,
            self.right - d.x,
            self.bottom - d.y,
        )
            .into()
    }

    #[must_use]
    pub fn with_outset(&self, d: impl Into<Vector>) -> Self {
        let d = d.into();
        (
            self.left - d.x,
            self.top - d.y,
            self.right + d.x,
            self.bottom + d.y,
        )
            .into()
    }

    pub fn intersects(&self, other: impl Into<Self>) -> bool {
        let other = other.into();
        let l = self.left.max(other.left);
        let r = self.right.min(other.right);
        let t = self.top.max(other.top);
        let b = self.bottom.min(other.bottom);
        l < r && t < b
    }

    pub fn joined(&self, other: impl Into<Self>) -> Self {
        let other = other.into();
        if other.is_empty() {
            return *self;
        }

        if self.is_empty() {
            return other;
        }

        (
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
            .into()
    }

    pub fn join(this: impl Into<Self>, other: impl Into<Self>) -> Self {
        this.into().joined(other)
    }

    pub fn rounded(&self) -> Self {
        (
            self.left.round(),
            self.top.round(),
            self.right.round(),
            self.bottom.round(),
        )
            .into()
    }

    pub fn rounded_in(&self) -> Self {
        (
            self.left.ceil(),
            self.top.ceil(),
            self.right.floor(),
            self.bottom.floor(),
        )
            .into()
    }

    pub fn rounded_out(&self) -> Self {
        (
            self.left.floor(),
            self.top.floor(),
            self.right.ceil(),
            self.bottom.ceil(),
        )
            .into()
    }

    pub fn sorted(&self) -> Self {
        (
            self.left.min(self.right),
            self.top.min(self.bottom),
            self.left.max(self.right),
            self.top.max(self.bottom),
        )
            .into()
    }

    pub fn to_scalars(&self) -> [f64; 4] {
        [self.left, self.top, self.right, self.bottom]
    }

    pub fn origin_and_size(&self) -> (Point, Size) {
        (self.origin(), self.size())
    }
}

impl From<(f64, f64, f64, f64)> for Rect {
    fn from((left, top, right, bottom): (f64, f64, f64, f64)) -> Self {
        (Point::new(left, top), Point::new(right, bottom)).into()
    }
}

impl From<(Point, Size)> for Rect {
    fn from((origin, size): (Point, Size)) -> Self {
        let rb = origin + size;
        (origin, rb).into()
    }
}

impl From<(Point, Point)> for Rect {
    // TODO: should we sort, guarantee that Rect is always normalized?
    fn from((origin, end): (Point, Point)) -> Self {
        Self {
            left: origin.x,
            top: origin.y,
            right: end.x,
            bottom: end.y,
        }
    }
}

impl From<Rect> for (Point, Point) {
    fn from(value: Rect) -> Self {
        (value.origin(), value.end())
    }
}

impl Add<Vector> for Rect {
    type Output = Self;

    fn add(self, d: Vector) -> Self::Output {
        Self {
            left: self.left + d.x,
            top: self.top + d.y,
            right: self.right + d.x,
            bottom: self.bottom + d.y,
        }
    }
}

impl Sub<Vector> for Rect {
    type Output = Self;

    fn sub(self, d: Vector) -> Self::Output {
        Self {
            left: self.left - d.x,
            top: self.top - d.y,
            right: self.right - d.x,
            bottom: self.bottom - d.y,
        }
    }
}

impl Centered for Rect {
    fn centered(&self) -> Self {
        *self - self.center()
    }
}

impl Contains<Point> for Rect {
    fn contains(&self, p: Point) -> bool {
        self.contains(&p)
    }
}

impl Contains<&Point> for Rect {
    fn contains(&self, p: &Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
}

impl Contains<&Rect> for Rect {
    fn contains(&self, r: &Rect) -> bool {
        !r.is_empty()
            && !self.is_empty()
            && self.left <= r.left
            && self.top <= r.top
            && self.right >= r.right
            && self.bottom >= r.bottom
    }
}
