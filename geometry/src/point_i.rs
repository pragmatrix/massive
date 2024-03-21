use std::ops::{Add, Neg, Sub};

use serde_tuple::{Deserialize_tuple, Serialize_tuple};

#[derive(Copy, Clone, PartialEq, Debug, Default, Serialize_tuple, Deserialize_tuple)]
pub struct PointI {
    pub x: i64,
    pub y: i64,
}

impl PointI {
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }

    pub fn length(&self) -> f64 {
        (self.squared_length() as f64).sqrt()
    }

    pub fn abs(&self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }

    pub fn squared_length(&self) -> i64 {
        self.x * self.x + self.y * self.y
    }
}

impl Neg for PointI {
    type Output = PointI;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y)
    }
}

impl Add for PointI {
    type Output = PointI;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for PointI {
    type Output = PointI;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl From<(i64, i64)> for PointI {
    fn from((x, y): (i64, i64)) -> Self {
        Self::new(x, y)
    }
}

impl From<(i32, i32)> for PointI {
    fn from((x, y): (i32, i32)) -> Self {
        Self::new(x as _, y as _)
    }
}
