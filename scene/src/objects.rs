use std::sync::Arc;

use massive_geometry::{Bounds, Transform};
use massive_shapes::{GlyphRun, Shape};

use crate::{Change, Handle, Id, Object, SceneChange};

/// A visual represents a set of shapes that have a common position / location in the space.
///
/// Architecture: This has now the same size as [`VisualRenderObj`]. Why not just clone this one for
/// the renderer then .. or even just the [`Handle<Visual>`]?
///
/// Detail: `Clone` was added for `Handle::update_with_if_changed()`.
#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub location: Handle<Location>,
    /// The current depth bias for this Visual. Default is 0, which renders it at first (without
    /// z-buffer) or with the corresponding depth bias (with z-buffer).
    pub depth_bias: usize,

    /// An optional clip bounds in model space 2D only.
    pub clip_bounds: Option<Bounds>,

    /// DR: Clients should be able to use [`Visual`] directly as a an abstract thing. Like for
    /// example a line which contains multiple Shapes (runs, quads, etc.). Therefore `Vec<Shape>`
    /// and not just `Shape`.
    ///
    /// DI: Another idea is to add `Shape::Combined(Vec<Shape>)`, but this makes extraction per
    /// renderer a bit more complex. This would also point to sharing Shapes as handles ... which
    /// could go in direction of layout?
    ///
    /// Arc is used here to make sharing shapes with the renderer really cheap.
    pub shapes: Arc<[Shape]>,
}

impl Visual {
    pub fn new(location: Handle<Location>, shapes: impl Into<Arc<[Shape]>>) -> Self {
        Self {
            location,
            depth_bias: 0,
            clip_bounds: None,
            shapes: shapes.into(),
        }
    }

    pub fn with_depth_bias(self, depth_bias: usize) -> Self {
        Self { depth_bias, ..self }
    }

    pub fn with_clip_bounds(self, bounds: impl Into<Bounds>) -> Self {
        Self {
            clip_bounds: Some(bounds.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct VisualRenderObj {
    pub location: Id,
    pub depth_bias: usize,
    pub clip_bounds: Option<Bounds>,
    pub shapes: Arc<[Shape]>,
}

impl VisualRenderObj {
    pub fn runs(&self) -> impl Iterator<Item = &GlyphRun> {
        self.shapes.iter().filter_map(|s| {
            if let Shape::GlyphRun(run) = s {
                Some(run)
            } else {
                None
            }
        })
    }
}

impl Object for Visual {
    // And upload the render shape.
    type Change = VisualRenderObj;

    fn to_change(&self) -> Self::Change {
        VisualRenderObj {
            location: self.location.id(),
            depth_bias: self.depth_bias,
            clip_bounds: self.clip_bounds,
            shapes: self.shapes.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Location {
    pub parent: Option<Handle<Location>>,
    pub transform: Handle<Transform>,
}

impl From<Handle<Transform>> for Location {
    fn from(transform: Handle<Transform>) -> Self {
        Self {
            parent: None,
            transform,
        }
    }
}

impl Location {
    pub fn new(parent: Option<Handle<Location>>, transform: impl Into<Handle<Transform>>) -> Self {
        Self {
            parent,
            transform: transform.into(),
        }
    }

    pub fn relative_to(mut self, parent: impl Into<Handle<Location>>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

// This allows `Into<Handle<Location>>` to take either a reference or an owned handle.
impl<T> From<&Handle<T>> for Handle<T>
where
    T: Object,
    SceneChange: From<Change<T::Change>>,
{
    fn from(value: &Handle<T>) -> Self {
        Handle::clone(value)
    }
}

impl Object for Location {
    type Change = LocationRenderObj;

    fn to_change(&self) -> Self::Change {
        let parent = self.parent.as_ref().map(|p| p.id());
        let transform = self.transform.id();
        LocationRenderObj { parent, transform }
    }
}

#[derive(Debug, Clone)]
pub struct LocationRenderObj {
    pub parent: Option<Id>,
    pub transform: Id,
}

impl Object for Transform {
    type Change = Self;

    fn to_change(&self) -> Self::Change {
        *self
    }
}
