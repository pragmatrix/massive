use std::ops::{Add, Div, Mul, Neg, Sub};

use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::Point3;

#[derive(Debug, Copy, Clone, PartialEq, Default, Serialize_tuple, Deserialize_tuple)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub type Vector = Point;

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn length(&self) -> f64 {
        self.squared_length().sqrt()
    }

    pub fn abs(&self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }

    /// Rotates the point around 0/0 (angle positive rotates to the right)
    /// <http://www.siggraph.org/education/materials/HyperGraph/modeling/mod_tran/2drota.htm>
    pub fn rotated_right(&self, angle: f64) -> Self {
        let (c, s) = (angle.cos(), angle.sin());
        let (x, y) = (self.x, self.y);
        Self::new(x * c - y * s, y * c + x * s)
    }

    pub fn scaled(&self, scaling: f64) -> Self {
        *self * scaling
    }

    pub fn squared_length(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    pub fn with_z(self, z: f64) -> Point3 {
        Point3::new(self.x, self.y, z)
    }
}

impl Neg for Point {
    type Output = Point;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y)
    }
}

impl Add for Point {
    type Output = Point;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Point;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f64> for Point {
    type Output = Point;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<f64> for Point {
    type Output = Point;

    fn div(self, rhs: f64) -> Self::Output {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl From<(f64, f64)> for Point {
    fn from((x, y): (f64, f64)) -> Self {
        Self::new(x, y)
    }
}

impl From<Point> for (f64, f64) {
    fn from(value: Point) -> Self {
        (value.x, value.y)
    }
}
