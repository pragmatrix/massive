//! Experimental traits that make it simpler to create handle objects more fluently.
//!
//! The primary content objects are focused on and the structure that surrounds it, is secondary and
//! added on top of them. This allows a more fluent API design.

use std::sync::Arc;

use massive_geometry::{Point, Transform};
use massive_shapes::{GlyphRun, Shape};

use crate::{Handle, Location, Visual};

// This should probably be moved to massive_geometry:

pub trait ToTransform {
    fn to_transform(&self) -> Transform;
}

impl ToTransform for Point {
    fn to_transform(&self) -> Transform {
        Transform::from_translation(self.with_z(0.0))
    }
}

pub trait ToLocation {
    fn to_location(&self) -> Location;
}

impl ToLocation for Handle<Transform> {
    fn to_location(&self) -> Location {
        Location::new(None, self.clone())
    }
}

pub trait IntoVisual {
    fn into_visual(self) -> VisualWithoutLocation;
}

impl IntoVisual for Shape {
    fn into_visual(self) -> VisualWithoutLocation {
        VisualWithoutLocation::new([self])
    }
}

impl IntoVisual for Option<Shape> {
    fn into_visual(self) -> VisualWithoutLocation {
        match self {
            Some(shape) => shape.into_visual(),
            None => [].into_visual(),
        }
    }
}

impl<const LEN: usize> IntoVisual for [Shape; LEN] {
    fn into_visual(self) -> VisualWithoutLocation {
        VisualWithoutLocation::new(self)
    }
}

#[derive(Debug)]
pub struct VisualWithoutLocation {
    pub shapes: Arc<[Shape]>,
}

impl VisualWithoutLocation {
    pub fn new(shapes: impl Into<Arc<[Shape]>>) -> Self {
        Self {
            shapes: shapes.into(),
        }
    }

    pub fn at(self, location: impl Into<Handle<Location>>) -> Visual {
        Visual::new(location.into(), self.shapes)
    }
}
