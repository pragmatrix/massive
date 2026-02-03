//! Experimental traits that make it simpler to create handle objects more fluently.
//!
//! The primary content objects are focused on and the structure that surrounds it, is secondary and
//! added on top of them. This allows a more fluent API design.

use std::sync::Arc;

use massive_geometry::{PixelCamera, Point, PointPx, Rect, Transform};
use massive_shapes::Shape;

use crate::{Handle, Location, Visual};

// This should probably be moved to massive_geometry:

pub trait ToTransform {
    fn to_transform(&self) -> Transform;
}

impl ToTransform for PointPx {
    fn to_transform(&self) -> Transform {
        let (x, y, z) = self.cast::<f64>().to_3d().into();
        (x, y, z).into()
    }
}

impl ToTransform for Point {
    fn to_transform(&self) -> Transform {
        Transform::from_translation(self.with_z(0.0))
    }
}

impl ToTransform for (f64, f64, f64) {
    fn to_transform(&self) -> Transform {
        Transform::from_translation(*self)
    }
}

impl ToTransform for Transform {
    fn to_transform(&self) -> Transform {
        *self
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

impl IntoVisual for Vec<Shape> {
    fn into_visual(self) -> VisualWithoutLocation {
        VisualWithoutLocation::new(self)
    }
}

impl IntoVisual for Arc<[Shape]> {
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

// Everything that can is positioned can be converted to a visual.
pub trait At {
    fn at(self, location: impl Into<Handle<Location>>) -> Visual;
}

impl<T> At for T
where
    T: IntoVisual,
{
    fn at(self, location: impl Into<Handle<Location>>) -> Visual {
        self.into_visual().at(location)
    }
}

pub trait ToCamera {
    fn to_camera(&self) -> PixelCamera;
}

impl<T> ToCamera for T
where
    T: ToTransform,
{
    fn to_camera(&self) -> PixelCamera {
        PixelCamera::look_at(self.to_transform(), None, PixelCamera::DEFAULT_FOVY)
    }
}

impl ToCamera for Rect {
    fn to_camera(&self) -> PixelCamera {
        self.center().to_camera().with_size(self.size())
    }
}
