use crate::Vector3;

// Plane defined by a point and outward normal.
pub struct Plane {
    pub point: Vector3,
    pub normal: Vector3,
}

impl Plane {
    pub fn new(point: impl Into<Vector3>, normal: impl Into<Vector3>) -> Self {
        Self {
            point: point.into(),
            normal: normal.into(),
        }
    }
}
