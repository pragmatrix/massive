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
pub struct PositionedShape {
    pub position: Handle<Position>,
    pub shape: Shape,
}

#[derive(Debug)]
pub struct PositionedRenderShape {
    pub position: Id,
    pub shape: Shape,
}

impl Object for PositionedShape {
    // We keep the position handle here.
    type Keep = Handle<Position>;
    // And upload the render shape.
    type Change = PositionedRenderShape;

    fn split(self) -> (Self::Keep, Self::Change) {
        let PositionedShape { position, shape } = self;
        let shape = PositionedRenderShape {
            position: position.id(),
            shape,
        };
        (position, shape)
    }

    fn promote_change(change: Change<Self::Change>) -> SceneChange {
        SceneChange::PositionedShape(change)
    }
}

impl PositionedShape {
    pub fn new(position: Handle<Position>, shape: impl Into<Shape>) -> Self {
        Self {
            position,
            shape: shape.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Position {
    pub parent: Option<Handle<Position>>,
    pub matrix: Handle<Matrix>,
}

impl From<Handle<Matrix>> for Position {
    fn from(matrix: Handle<Matrix>) -> Self {
        Position {
            parent: None,
            matrix,
        }
    }
}

impl Object for Position {
    type Keep = Self;
    type Change = PositionRenderObj;

    fn promote_change(change: Change<Self::Change>) -> SceneChange {
        SceneChange::Position(change)
    }

    fn split(self) -> (Self::Keep, Self::Change) {
        let parent = self.parent.as_ref().map(|p| p.id());
        let matrix = self.matrix.id();
        (self, PositionRenderObj { parent, matrix })
    }
}

#[derive(Debug)]
pub struct PositionRenderObj {
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
    use crate::{Director, Position, PositionedShape, SceneChange};
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
            // Keep the positioned shapes until the director ran through.
            let _positioned = into_positioned_shapes(&mut director, shapes);
            director.action()?;
        }

        Ok(channel_rx.try_recv().unwrap_or_default())
    }

    pub fn into_positioned_shapes(
        director: &mut Director,
        shapes: Vec<Shape>,
    ) -> Vec<Handle<PositionedShape>> {
        let mut position_handles: HashMap<*const Matrix4, Handle<Position>> = HashMap::new();
        let mut positioned_shapes = Vec::with_capacity(shapes.len());

        for shape in shapes {
            let matrix = match &shape {
                Shape::GlyphRun(GlyphRunShape { model_matrix, .. }) => model_matrix,
                Shape::Quads(QuadsShape { model_matrix, .. }) => model_matrix,
            };

            let position = position_handles.entry(Rc::as_ptr(matrix)).or_insert_with(
                || -> Handle<Position> {
                    let matrix = director.cast(**matrix);
                    director.cast(matrix.into())
                },
            );

            let positioned = match shape {
                Shape::GlyphRun(GlyphRunShape { run, .. }) => {
                    PositionedShape::new(position.clone(), run)
                }
                Shape::Quads(QuadsShape { quads, .. }) => {
                    PositionedShape::new(position.clone(), quads)
                }
            };

            positioned_shapes.push(director.cast(positioned));
        }

        positioned_shapes
    }
}
