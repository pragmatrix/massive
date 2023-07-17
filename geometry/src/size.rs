use std::ops;

use crate::Point;

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

impl From<(f64, f64)> for Size {
    fn from((width, height): (f64, f64)) -> Self {
        Size::new(width, height)
    }
}

impl ops::Mul<f64> for Size {
    type Output = Size;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.width * rhs, self.height * rhs)
    }
}

impl ops::Div<f64> for Size {
    type Output = Size;

    fn div(self, rhs: f64) -> Self::Output {
        Self::new(self.width / rhs, self.height / rhs)
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
