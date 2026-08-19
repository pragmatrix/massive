use std::ops;

use crate::{Point, Rect, SizePx};

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    pub fn is_empty(&self) -> bool {
        !(self.width > 0.0 && self.height > 0.0)
    }

    pub fn to_rect(&self) -> Rect {
        Rect::new(Point::ORIGIN, *self)
    }

    pub fn min_element(&self) -> f64 {
        self.width.min(self.height)
    }

    pub fn max_element(&self) -> f64 {
        self.width.max(self.height)
    }

    pub fn center(&self) -> Point {
        Point::new(self.width * 0.5, self.height * 0.5)
    }

    pub fn aspect_ratio(&self) -> f64 {
        self.width / self.height
    }
}

impl From<(f64, f64)> for Size {
    fn from((width, height): (f64, f64)) -> Self {
        Size::new(width, height)
    }
}

impl From<SizePx> for Size {
    fn from(size: SizePx) -> Self {
        Size::new(size.width as f64, size.height as f64)
    }
}

impl ops::Mul<f64> for Size {
    type Output = Size;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.width * rhs, self.height * rhs)
    }
}

impl ops::Mul<Size> for Size {
    type Output = Size;

    fn mul(self, rhs: Size) -> Self::Output {
        Self::new(self.width * rhs.width, self.height * rhs.height)
    }
}

impl ops::Div<f64> for Size {
    type Output = Size;

    fn div(self, rhs: f64) -> Self::Output {
        Self::new(self.width / rhs, self.height / rhs)
    }
}

impl ops::Div<Size> for Size {
    type Output = Size;

    fn div(self, rhs: Size) -> Self::Output {
        Self::new(self.width / rhs.width, self.height / rhs.height)
    }
}

impl ops::Add for Size {
    type Output = Size;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.width + rhs.width, self.height + rhs.height)
    }
}

impl ops::Sub for Size {
    type Output = Size;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.width - rhs.width, self.height - rhs.height)
    }
}

impl ops::Add<Size> for Point {
    type Output = Point;

    fn add(self, rhs: Size) -> Self::Output {
        Point::new(self.x + rhs.width, self.y + rhs.height)
    }
}

impl ops::Sub<Size> for Point {
    type Output = Point;

    fn sub(self, rhs: Size) -> Self::Output {
        Point::new(self.x - rhs.width, self.y - rhs.height)
    }
}
