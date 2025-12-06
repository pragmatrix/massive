use std::ops;

use crate::Vector3;

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Size3(Vector3);

impl Size3 {
    pub const fn new(v: Vector3) -> Self {
        Self(v)
    }
}

impl From<Vector3> for Size3 {
    fn from(v: Vector3) -> Self {
        Self::new(v)
    }
}

impl From<(f64, f64, f64)> for Size3 {
    fn from((x, y, z): (f64, f64, f64)) -> Self {
        Self::new((x, y, z).into())
    }
}

impl ops::Mul<f64> for Size3 {
    type Output = Size3;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.0 * rhs)
    }
}

impl ops::Div<f64> for Size3 {
    type Output = Size3;

    fn div(self, rhs: f64) -> Self::Output {
        Self::new(self.0 / rhs)
    }
}

impl ops::Add<Size3> for Size3 {
    type Output = Size3;

    fn add(self, rhs: Size3) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl ops::Sub<Size3> for Size3 {
    type Output = Size3;

    fn sub(self, rhs: Size3) -> Self::Output {
        Self::new(self.0 - rhs.0)
    }
}

impl ops::Add<Size3> for Vector3 {
    type Output = Self;

    fn add(self, rhs: Size3) -> Self::Output {
        Self::new(self.x + rhs.0.x, self.y + rhs.0.y, self.z + rhs.0.z)
    }
}

impl ops::Sub<Size3> for Vector3 {
    type Output = Self;

    fn sub(self, rhs: Size3) -> Self::Output {
        Self::new(self.x - rhs.0.x, self.y - rhs.0.y, self.z - rhs.0.z)
    }
}

impl From<Size3> for Vector3 {
    fn from(size: Size3) -> Self {
        size.0
    }
}
