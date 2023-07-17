use crate::{Point3, Size3};

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bounds3 {
    pub min: Point3,
    pub max: Point3,
}

impl Bounds3 {
    pub fn new(min: Point3, max: Point3) -> Self {
        Self { min, max }
    }

    pub fn size(&self) -> Size3 {
        let v = self.max - self.min;
        Size3::new((v.x, v.y, v.z).into())
    }
}
