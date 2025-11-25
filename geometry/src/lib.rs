//! Geometry primitives, taken from the BrainSharper project at 20230701

mod bezier_algorithms;
mod bounds;
mod bounds3;
mod camera;
mod color;
mod cubic_bezier;
mod depth_range;
mod flo_curves;
mod line;
mod plane;
mod point;
mod point_i;
mod projection;
mod ray;
mod rect;
mod size;
mod size3;
mod size_i;
mod unit_interval;

pub use bounds::*;
pub use bounds3::*;
pub use camera::*;
use cgmath::One;
pub use color::*;
pub use cubic_bezier::*;
pub use depth_range::*;
pub use line::*;
pub use plane::*;
pub use point::*;
pub use point_i::*;
pub use projection::*;
pub use ray::*;
pub use rect::*;
pub use size::*;
pub use size3::*;
pub use size_i::*;
pub use unit_interval::*;

pub trait Centered {
    fn centered(&self) -> Self;
}

pub const EPSILON: f64 = f64::EPSILON;

pub trait Contains<Other> {
    fn contains(&self, other: Other) -> bool;
}

// Performance: This should probably not Copy!
pub type Matrix4 = cgmath::Matrix4<f64>;
pub type Point3 = cgmath::Point3<f64>;
pub type Vector3 = cgmath::Vector3<f64>;
pub type Vector4 = cgmath::Vector4<f64>;

pub trait Identity {
    fn identity() -> Self;
}

impl Identity for Matrix4 {
    fn identity() -> Self {
        cgmath::Matrix4::one()
    }
}

pub trait PerspectiveDivide {
    fn perspective_divide(&self) -> Option<Point3>;
}

impl PerspectiveDivide for Vector4 {
    // Perspective divide helper: converts homogeneous Vector4 (x,y,z,w) into Point3 (x/w,y/w,z/w)
    // returning None if w is too close to zero.
    fn perspective_divide(&self) -> Option<Point3> {
        let w = self.w;
        if w.abs() < EPSILON {
            return None;
        }
        Some(Point3::new(self.x / w, self.y / w, self.z / w))
    }
}
