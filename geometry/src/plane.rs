use crate::{Point3, Vector3};

// Plane defined by a point and outward normal.
pub struct Plane {
    pub point: Point3,
    pub normal: Vector3,
}

impl Plane {
    pub fn new(point: impl Into<Point3>, normal: impl Into<Vector3>) -> Self {
        Self {
            point: point.into(),
            normal: normal.into(),
        }
    }
}
