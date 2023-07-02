//! Geometry primitives, taken from the BrainSharper project at 20230701

mod bezier_algorithms;
mod bounds;
mod color;
mod cubic_bezier;
mod flo_curves;
mod line;
mod point;
mod rect;
mod size;
mod unit_interval;

pub use bounds::*;
pub use color::*;
pub use cubic_bezier::*;
pub use line::*;
pub use point::*;
pub use rect::*;
pub use size::Size;
pub use unit_interval::*;

#[allow(non_camel_case_types)]
pub type scalar = f64;

pub trait Centered {
    fn centered(&self) -> Self;
}

pub trait Contains<Other> {
    fn contains(&self, other: Other) -> bool;
}
