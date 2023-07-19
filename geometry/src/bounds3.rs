use crate::{Point3, Size3};

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bounds3 {
    pub min: Point3,
    pub max: Point3,
}

impl Bounds3 {
    pub fn new(min: impl Into<Point3>, max: impl Into<Point3>) -> Self {
        Self {
            min: min.into(),
            max: max.into(),
        }
    }

    pub fn size(&self) -> Size3 {
        let v = self.max - self.min;
        Size3::new((v.x, v.y, v.z).into())
    }
}
