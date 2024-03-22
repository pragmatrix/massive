use std::ops;

use crate::PointI;

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct SizeI {
    pub width: u64,
    pub height: u64,
}

impl SizeI {
    pub const fn new(width: u64, height: u64) -> Self {
        Self { width, height }
    }
}

impl From<(u32, u32)> for SizeI {
    fn from((width, height): (u32, u32)) -> Self {
        SizeI::new(width as _, height as _)
    }
}

impl From<(u64, u64)> for SizeI {
    fn from((width, height): (u64, u64)) -> Self {
        SizeI::new(width, height)
    }
}

impl ops::Mul<u64> for SizeI {
    type Output = SizeI;

    fn mul(self, rhs: u64) -> Self::Output {
        Self::new(self.width * rhs, self.height * rhs)
    }
}

impl ops::Add<SizeI> for PointI {
    type Output = PointI;

    fn add(self, rhs: SizeI) -> Self::Output {
        PointI::new(self.x + rhs.width as i64, self.y + rhs.height as i64)
    }
}

impl ops::Sub<SizeI> for PointI {
    type Output = PointI;

    fn sub(self, rhs: SizeI) -> Self::Output {
        PointI::new(self.x - rhs.width as i64, self.y - rhs.height as i64)
    }
}
