use crate::{Size3, Vector3};

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bounds3 {
    pub min: Vector3,
    pub max: Vector3,
}

impl Bounds3 {
    pub fn new(min: impl Into<Vector3>, max: impl Into<Vector3>) -> Self {
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
