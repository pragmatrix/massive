use crate::{Plane, Vector3, EPSILON};

// Ray in 3D space.
#[derive(Debug, Clone)]
pub struct Ray {
    pub origin: Vector3,
    pub dir: Vector3,
}

impl Ray {
    pub fn new(origin: impl Into<Vector3>, dir: impl Into<Vector3>) -> Self {
        Self {
            origin: origin.into(),
            dir: dir.into(),
        }
    }

    pub fn from_points(origin: impl Into<Vector3>, target: impl Into<Vector3>) -> Option<Self> {
        let origin = origin.into();
        let target = target.into();

        let mut dir = target - origin;
        if dir.length_squared() < EPSILON * 1e-6 {
            return None;
        }
        dir = dir.normalize();
        Some(Self::new(origin, dir))
    }

    pub fn intersect_plane(&self, plane: &Plane) -> Option<Vector3> {
        let denom = plane.normal.dot(self.dir);
        if denom.abs() < EPSILON {
            return None;
        }
        let t = plane.normal.dot(plane.point - self.origin) / denom;
        if t < 0.0 {
            return None;
        }
        Some(self.origin + self.dir * t)
    }
}
