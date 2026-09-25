//! Experimental traits that make it simpler to create handle objects more fluently.
//!
//! The primary content objects are focused on and the structure that surrounds it, is secondary and
//! added on top of them. This allows a more fluent API design.

use std::sync::Arc;

use massive_geometry::{PixelCamera, Point, PointPx, Transform};
use massive_shapes::Shape;

use crate::scene::enter_pair;
use crate::{AnyCollector, Handle, Location, LocationParent, LocationSpace, Ref, Visual};

// This should probably be moved to massive_geometry:

/// Converts a value into a [`Transform`].
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

/// Converts a transform handle into a world-rooted [`Location`].
pub trait ToLocation {
    fn to_location(&self) -> Location;
}

impl ToLocation for Handle<Transform> {
    fn to_location(&self) -> Location {
        Location::new(LocationSpace::World, self.clone())
    }
}

/// A location that has not entered a scene yet. Enter it to create a location with an
/// initially-identity transform, returning both handles so the transform can be updated later.
#[derive(Debug)]
#[must_use = "the location is not entered until `.enter_in(collector)` is called"]
pub struct UnenteredLocation {
    parent: LocationParent,
}

impl UnenteredLocation {
    /// Root the location in the given coordinate space.
    pub fn in_space(mut self, space: LocationSpace) -> Self {
        self.parent = space.into();
        self
    }

    /// Make the location a child of the given parent location.
    pub fn relative_to(mut self, parent: impl Into<Ref<Location>>) -> Self {
        self.parent = parent.into().into();
        self
    }

    /// Enter a location with an initially-identity transform into `collector`, returning both
    /// handles.
    ///
    /// The transform and the location are published as one batch, so the queue's lock is acquired
    /// once instead of once per object.
    pub fn enter_in(self, collector: &AnyCollector) -> (Handle<Transform>, Handle<Location>) {
        enter_pair(collector, Transform::IDENTITY, |transform| {
            Location::new(self.parent, transform.clone())
        })
    }
}

/// Creates an unentered location whose transform starts as identity.
pub fn identity_location() -> UnenteredLocation {
    UnenteredLocation {
        parent: LocationSpace::World.into(),
    }
}
/// Converts a value into a [`VisualWithoutLocation`].
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

/// Shapes that are not yet placed at a location.
#[derive(Debug)]
#[must_use = "the visual is not placed until `.at(location)` is called"]
pub struct VisualWithoutLocation {
    pub shapes: Arc<[Shape]>,
}

impl VisualWithoutLocation {
    pub fn new(shapes: impl Into<Arc<[Shape]>>) -> Self {
        Self {
            shapes: shapes.into(),
        }
    }

    #[must_use = "the visual is not placed until it is `.enter()`ed"]
    pub fn at(self, location: impl Into<Ref<Location>>) -> Visual {
        Visual::new(location.into(), self.shapes)
    }
}

/// Places a value at a location, converting it into a [`Visual`].
pub trait At {
    #[must_use = "the visual is not entered until `.enter()` is called"]
    fn at(self, location: impl Into<Ref<Location>>) -> Visual;
}

impl<T> At for T
where
    T: IntoVisual,
{
    fn at(self, location: impl Into<Ref<Location>>) -> Visual {
        self.into_visual().at(location)
    }
}

/// Converts a value into a [`PixelCamera`].
pub trait ToCamera {
    fn to_camera(&self) -> PixelCamera;
}

impl<T> ToCamera for T
where
    T: ToTransform,
{
    fn to_camera(&self) -> PixelCamera {
        PixelCamera::look_at(
            self.to_transform(),
            PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY),
            PixelCamera::DEFAULT_FOVY,
        )
    }
}
