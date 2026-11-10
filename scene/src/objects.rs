use std::sync::Arc;

use massive_geometry::{Bounds, Transform};
use massive_shapes::Shape;

use crate::handle::sealed::Sealed;
use crate::lifetime_stage::{Kind, Targets};
use crate::{Change, Handle, Id, Object, Ref, RenderChange, RenderSceneChange, SceneChange};

/// The coordinate space root a location hierarchy starts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationSpace {
    /// Pixel coordinates as expressed by the desktop model.
    World,
    /// Pixel coordinates relative to the camera, independent of the world camera transform.
    Camera,
}

/// The coordinate space root a location hierarchy starts in, or the parent location a hierarchy
/// continues from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationParent {
    /// The location is the root of a hierarchy in the given coordinate space.
    Root(LocationSpace),
    /// The location is a child of another location and inherits its space.
    Location(Id<Location>),
}

impl From<LocationSpace> for LocationParent {
    fn from(space: LocationSpace) -> Self {
        Self::Root(space)
    }
}

impl From<Id<Location>> for LocationParent {
    fn from(location: Id<Location>) -> Self {
        Self::Location(location)
    }
}

impl From<&Handle<Location>> for LocationParent {
    fn from(location: &Handle<Location>) -> Self {
        Self::Location(location.into())
    }
}

impl From<&Ref<Location>> for LocationParent {
    fn from(location: &Ref<Location>) -> Self {
        Self::Location(location.into())
    }
}

/// The final, resolved state of a location: composed transform, inherited alpha sanitized into
/// `[0, 1]`, and the space of its root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedLocation {
    pub transform: Transform,
    pub alpha: f32,
    pub space: LocationSpace,
}

/// A visual represents a set of shapes that have a common position / location in the space.
///
/// The renderer receives the visual itself with every change, so it refers to its location by id
/// (ADR 0040).
#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub location: Id<Location>,
    /// Optional decal ordering value for this visual.
    ///
    /// If set, the renderer treats this visual as a decal and renders it in decal order using the
    /// decal pipeline configuration.
    ///
    /// Decals are drawn with depth testing but without z-buffer writes.
    ///
    /// Decal layers render after non-decal visuals in ascending order, so `decal_order = 0` is
    /// the first decal layer.
    pub decal_order: Option<usize>,

    /// An optional clip bounds in model space 2D only.
    pub clip_bounds: Option<Bounds>,

    /// DR: Clients should be able to use [`Visual`] directly as an abstract thing. Like for
    /// example a line which contains multiple Shapes (runs, quads, etc.). Therefore, `Vec<Shape>`
    /// and not just `Shape`.
    ///
    /// DI: Another idea is to add `Shape::Combined(Vec<Shape>)`, but this makes extraction per
    /// renderer a bit more complex. This would also point to sharing Shapes as handles, which
    /// could go in direction of layout?
    ///
    /// Arc is used here to make sharing shapes with the renderer really cheap.
    pub shapes: Arc<[Shape]>,
}

impl Visual {
    pub fn new(location: impl Into<Id<Location>>, shapes: impl Into<Arc<[Shape]>>) -> Self {
        Self {
            location: location.into(),
            decal_order: None,
            clip_bounds: None,
            shapes: shapes.into(),
        }
    }

    /// Marks this visual as a decal and assigns its decal layer order.
    ///
    /// Calling this opts the visual into decal rendering (including the decal pipeline depth
    /// bias). The numeric value only controls ordering among decal visuals where `0` is the first
    /// decal layer rendered after non-decal visuals; it does not change the bias magnitude.
    pub fn with_decal_order(self, decal_order: usize) -> Self {
        Self {
            decal_order: Some(decal_order),
            ..self
        }
    }

    /// Accepts an `Option`, so a visual that is clipped only sometimes is built in one chain.
    pub fn with_clip_bounds(self, clip_bounds: impl Into<Option<Bounds>>) -> Self {
        Self {
            clip_bounds: clip_bounds.into(),
            ..self
        }
    }
}

/// The setters of a submitted visual. Each one sends the update only when the field changes.
impl Handle<Visual> {
    pub fn set_shapes(&mut self, shapes: Arc<[Shape]>) {
        self.set_if_changed(|visual| &mut visual.shapes, shapes);
    }

    pub fn set_clip_bounds(&mut self, clip_bounds: Option<Bounds>) {
        self.set_if_changed(|visual| &mut visual.clip_bounds, clip_bounds);
    }
}

impl Sealed for Visual {}

impl Object for Visual {
    const KIND: Kind = Kind::Visual;

    fn targets(&self) -> Targets {
        Targets::new([Some(self.location.into()), None])
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Visual(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Visual(change)
    }
}

/// A location places what refers to it: its transform applies relative to its parent.
///
/// The renderer receives the location itself with every change, so it refers to its parent and
/// its transform by id (ADR 0040).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub parent: LocationParent,
    pub transform: Id<Transform>,
    /// Alpha as set; consumers sanitize it into `[0, 1]` because it is a plain writable field.
    pub alpha: f32,
}

impl Location {
    pub fn new(parent: impl Into<LocationParent>, transform: impl Into<Id<Transform>>) -> Self {
        Self {
            parent: parent.into(),
            transform: transform.into(),
            alpha: 1.0,
        }
    }

    /// A root location in the given coordinate space.
    pub fn root(space: LocationSpace, transform: impl Into<Id<Transform>>) -> Self {
        Self::new(space, transform)
    }

    pub fn relative_to(mut self, parent: impl Into<LocationParent>) -> Self {
        self.parent = parent.into();
        self
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    /// Scene changes can be constructed directly, so the renderer must not trust their alpha.
    pub fn sanitized_alpha(&self) -> f32 {
        sanitize_alpha(self.alpha)
    }
}

/// The setters of a submitted location. Each one sends the update only when the field changes.
impl Handle<Location> {
    pub fn set_parent(&mut self, parent: impl Into<LocationParent>) {
        self.set_if_changed(|location| &mut location.parent, parent.into());
    }

    pub fn set_alpha(&mut self, alpha: f32) {
        self.set_if_changed(|location| &mut location.alpha, alpha);
    }
}

impl Sealed for Location {}

impl Object for Location {
    const KIND: Kind = Kind::Location;

    fn targets(&self) -> Targets {
        let parent = match self.parent {
            LocationParent::Location(parent) => Some(parent.into()),
            LocationParent::Root(_) => None,
        };
        Targets::new([Some(self.transform.into()), parent])
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Location(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Location(change)
    }
}

/// Clamp an alpha into `[0, 1]`, treating a non-finite value as opaque.
///
/// A location's alpha is stored as set, so every consumer of it must sanitize.
fn sanitize_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

impl Sealed for Transform {}

impl Object for Transform {
    const KIND: Kind = Kind::Transform;

    fn targets(&self) -> Targets {
        Targets::default()
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Transform(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Transform(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::current_queue::test_queue::with_test_queue;

    #[test]
    fn location_new_defaults_to_opaque_alpha() {
        with_test_queue(|| {
            let transform = crate::submit(Transform::IDENTITY);
            let location = Location::root(LocationSpace::World, &transform);

            assert_eq!(location.alpha, 1.0);
        });
    }

    #[test]
    fn location_alpha_is_stored_as_set() {
        with_test_queue(|| {
            let transform = crate::submit(Transform::IDENTITY);
            let location = Location::root(LocationSpace::World, &transform).with_alpha(2.0);

            assert_eq!(location.alpha, 2.0);
        });
    }
}
