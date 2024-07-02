use derive_more::From;

use crate::{Change, Handle, Id, Object, SceneChange};
use massive_geometry as geometry;
use massive_shapes::{GlyphRun, Quads};

#[derive(Debug, From)]
pub enum Shape {
    GlyphRun(GlyphRun),
    Quads(Quads),
}

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
    pub shapes: Vec<Shape>,
}

#[derive(Debug)]
pub struct VisualRenderObj {
    pub position: Id,
    pub shapes: Vec<Shape>,
}

impl Object for Visual {
    // We keep the position handle here.
    type Keep = Handle<Location>;
    // And upload the render shape.
    type Change = VisualRenderObj;

    fn split(self) -> (Self::Keep, Self::Change) {
        let Visual {
            location: position,
            shapes,
        } = self;
        let shape = VisualRenderObj {
            position: position.id(),
            shapes,
        };
        (position, shape)
    }

    fn promote_change(change: Change<Self::Change>) -> SceneChange {
        SceneChange::Visual(change)
    }
}

impl Visual {
    pub fn new(location: Handle<Location>, shapes: impl Into<Vec<Shape>>) -> Self {
        Self {
            location,
            shapes: shapes.into(),
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
        Location {
            parent: None,
            matrix,
        }
    }
}

impl Object for Location {
    type Keep = Self;
    type Change = LocationRenderObj;

    fn promote_change(change: Change<Self::Change>) -> SceneChange {
        SceneChange::Location(change)
    }

    fn split(self) -> (Self::Keep, Self::Change) {
        let parent = self.parent.as_ref().map(|p| p.id());
        let matrix = self.matrix.id();
        (self, LocationRenderObj { parent, matrix })
    }
}

#[derive(Debug)]
pub struct LocationRenderObj {
    pub parent: Option<Id>,
    pub matrix: Id,
}

pub type Matrix = geometry::Matrix4;

impl Object for Matrix {
    type Keep = ();
    type Change = Self;

    fn split(self) -> (Self::Keep, Self::Change) {
        ((), self)
    }

    fn promote_change(change: Change<Self::Change>) -> SceneChange {
        SceneChange::Matrix(change)
    }
}

pub mod legacy {
    use super::Handle;
    use crate::{Director, Location, SceneChange, Visual};
    use anyhow::Result;
    use massive_geometry::Matrix4;
    use massive_shapes::{GlyphRunShape, QuadsShape, Shape};
    use std::{collections::HashMap, rc::Rc};
    use tokio::sync::mpsc;

    pub fn bootstrap_scene_changes(shapes: Vec<Shape>) -> Result<Vec<SceneChange>> {
        let (channel_tx, mut channel_rx) = mpsc::channel(1);

        let mut director = Director::from_sender(channel_tx);

        // The shapes must be alive

        {
            // Keep the visuals until the director ran through.
            let _visuals = into_visuals(&mut director, shapes);
            director.action()?;
        }

        Ok(channel_rx.try_recv().unwrap_or_default())
    }

    pub fn into_visuals(director: &mut Director, shapes: Vec<Shape>) -> Vec<Handle<Visual>> {
        let mut location_handles: HashMap<*const Matrix4, Handle<Location>> = HashMap::new();
        let mut visuals = Vec::with_capacity(shapes.len());

        for shape in shapes {
            let matrix = match &shape {
                Shape::GlyphRun(GlyphRunShape { model_matrix, .. }) => model_matrix,
                Shape::Quads(QuadsShape { model_matrix, .. }) => model_matrix,
            };

            let position = location_handles.entry(Rc::as_ptr(matrix)).or_insert_with(
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
