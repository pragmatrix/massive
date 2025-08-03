use std::sync::Arc;

use derive_more::From;

use crate::{Handle, Id, Object};
use massive_geometry as geometry;
use massive_shapes::{GlyphRun, Quads};

#[derive(Debug, Clone, From)]
pub enum Shape {
    GlyphRun(GlyphRun),
    Quads(Quads),
}

/// A visual represents a set of shapes that have a common position / location in the space.
#[derive(Debug)]
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
    /// Arc to make sharing shapes with the renderer really cheap. Cloning them would be too heavy.
    pub shapes: Arc<[Shape]>,
}

#[derive(Debug)]
pub struct VisualRenderObj {
    pub location: Id,
    pub shapes: Arc<[Shape]>,
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

impl Visual {
    pub fn new(location: Handle<Location>, shapes: impl Into<Vec<Shape>>) -> Self {
        Self {
            location,
            shapes: shapes.into().into(),
        }
    }
}

impl From<Shape> for Vec<Shape> {
    fn from(value: Shape) -> Self {
        vec![value]
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

impl Object for Location {
    type Change = LocationRenderObj;

    fn to_change(&self) -> Self::Change {
        let parent = self.parent.as_ref().map(|p| p.id());
        let matrix = self.matrix.id();
        LocationRenderObj { parent, matrix }
    }
}

#[derive(Debug)]
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

pub mod legacy {
    use super::Handle;
    use crate::{Location, Scene, Visual};
    use massive_geometry::Matrix4;
    use massive_shapes::{GlyphRunShape, QuadsShape, Shape};
    use std::{collections::HashMap, sync::Arc};

    pub fn into_visuals(director: &Scene, shapes: Vec<Shape>) -> Vec<Handle<Visual>> {
        let mut location_handles: HashMap<*const Matrix4, Handle<Location>> = HashMap::new();
        let mut visuals = Vec::with_capacity(shapes.len());

        for shape in shapes {
            let matrix = match &shape {
                Shape::GlyphRun(GlyphRunShape { model_matrix, .. }) => model_matrix,
                Shape::Quads(QuadsShape { model_matrix, .. }) => model_matrix,
            };

            let position = location_handles.entry(Arc::as_ptr(matrix)).or_insert_with(
                || -> Handle<Location> {
                    let matrix = director.stage(**matrix);
                    director.stage(matrix.into())
                },
            );

            let visual = match shape {
                Shape::GlyphRun(GlyphRunShape { run, .. }) => {
                    Visual::new(position.clone(), super::Shape::from(run))
                }
                Shape::Quads(QuadsShape { quads, .. }) => {
                    Visual::new(position.clone(), super::Shape::from(quads))
                }
            };

            visuals.push(director.stage(visual));
        }

        visuals
    }
}
