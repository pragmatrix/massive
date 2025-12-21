//! Geometry primitives, taken from the BrainSharper project at 20230701

mod bezier_algorithms;
mod bounds;
mod bounds3;
mod color;
mod cubic_bezier;
mod depth_range;
mod flo_curves;
mod line;
mod pixel_camera;
mod plane;
mod point;
pub mod prelude;
mod projection;
mod ray;
mod rect;
mod size;
mod size3;
mod transform;
mod unit_interval;

pub use bounds::*;
pub use bounds3::*;
pub use color::*;
pub use cubic_bezier::*;
pub use depth_range::*;
pub use line::*;
pub use pixel_camera::*;
pub use plane::*;
pub use point::*;
pub use projection::*;
pub use ray::*;
pub use rect::*;
pub use size::*;
pub use size3::*;
pub use transform::*;
pub use unit_interval::*;

pub trait Centered {
    fn centered(&self) -> Self;
}

pub const EPSILON: f64 = f64::EPSILON;

pub trait Contains<Other> {
    fn contains(&self, other: Other) -> bool;
}

// Performance: This should probably not Copy!
pub type Matrix4 = glam::DMat4;
pub type Vector3 = glam::DVec3;
pub type Vector4 = glam::DVec4;
pub type Quaternion = glam::DQuat;

pub trait PerspectiveDivide {
    fn perspective_divide(&self) -> Option<Vector3>;
}

impl PerspectiveDivide for Vector4 {
    // Perspective divide helper: converts homogeneous Vector4 (x,y,z,w) into Vector3 (x/w,y/w,z/w)
    // returning None if w is too close to zero.
    fn perspective_divide(&self) -> Option<Vector3> {
        let w = self.w;
        if w.abs() < EPSILON {
            return None;
        }
        Some(Vector3::new(self.x / w, self.y / w, self.z / w))
    }
}

pub struct PixelUnit;
pub type SizePx = euclid::Size2D<u32, PixelUnit>;
pub type VectorPx = euclid::Vector2D<i32, PixelUnit>;
// This go introduced because Box2D::contains needs a Point and not a vector.
pub type PointPx = euclid::Point2D<i32, PixelUnit>;
pub type BoxPx = euclid::Box2D<i32, PixelUnit>;

pub trait CastSigned {
    type SignedType;

    fn cast_signed(&self) -> Self::SignedType;
}

impl<U> CastSigned for euclid::Size2D<u32, U> {
    type SignedType = euclid::Vector2D<i32, U>;

    fn cast_signed(&self) -> Self::SignedType {
        self.cast().to_vector()
    }
}

pub trait ToVector3 {
    fn to_vector3(self) -> Vector3;
}

impl<U> ToVector3 for euclid::Vector3D<f64, U> {
    fn to_vector3(self) -> Vector3 {
        let (x, y, z) = self.into();
        (x, y, z).into()
    }
}
