use std::sync::Arc;

use crate::{Handle, Id, Object};
use massive_geometry as geometry;
use massive_shapes::{GlyphRun, Shape};

/// A visual represents a set of shapes that have a common position / location in the space.
///
/// Architecture: This has now the same size as [`VisualRenderObj`]. Why not just clone this one for
/// the renderer then .. or even just the [`Handle<Visual>`]?
#[derive(Debug, PartialEq)]
pub struct Visual {
    pub location: Handle<Location>,
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
            shapes: shapes.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VisualRenderObj {
    pub location: Id,
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
            shapes: self.shapes.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Location {
    pub parent: Option<Handle<Location>>,
    pub matrix: Handle<Matrix>,
}

impl From<Handle<Matrix>> for Location {
    fn from(matrix: Handle<Matrix>) -> Self {
        Self {
            parent: None,
            matrix,
        }
    }
}

impl Location {
    pub fn new(parent: Option<Handle<Location>>, matrix: Handle<Matrix>) -> Self {
        Self { parent, matrix }
    }
}

impl Object for Location {
    type Change = LocationRenderObj;

    fn to_change(&self) -> Self::Change {
        let parent = self.parent.as_ref().map(|p| p.id());
        let matrix = self.matrix.id();
        LocationRenderObj { parent, matrix }
    }
}

#[derive(Debug, Clone)]
pub struct LocationRenderObj {
    pub parent: Option<Id>,
    pub matrix: Id,
}

pub type Matrix = geometry::Matrix4;

impl Object for Matrix {
    type Change = Self;

    fn to_change(&self) -> Self::Change {
        *self
    }
}
