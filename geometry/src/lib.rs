//! Geometry primitives, taken from the BrainSharper project at 20230701

mod bezier_algorithms;
mod bounds;
mod bounds3;
mod camera;
mod color;
mod cubic_bezier;
mod flo_curves;
mod line;
mod point;
mod point_i;
mod projection;
mod rect;
mod size;
mod size3;
mod unit_interval;

pub use bounds::*;
pub use bounds3::*;
pub use camera::*;
use cgmath::One;
pub use color::*;
pub use cubic_bezier::*;
pub use line::*;
pub use point::*;
pub use point_i::*;
pub use projection::*;
pub use rect::*;
pub use size::*;
pub use size3::*;
pub use unit_interval::*;

#[allow(non_camel_case_types)]
pub type scalar = f64;

pub trait Centered {
    fn centered(&self) -> Self;
}

pub trait Contains<Other> {
    fn contains(&self, other: Other) -> bool;
}

pub type Matrix4 = cgmath::Matrix4<f64>;
pub type Point3 = cgmath::Point3<f64>;
pub type Vector3 = cgmath::Vector3<f64>;

pub trait Identity {
    fn identity() -> Self;
}

impl Identity for Matrix4 {
    fn identity() -> Self {
        cgmath::Matrix4::one()
    }
}
