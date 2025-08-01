use std::any::TypeId;

use derive_more::From;
use massive_geometry::Matrix4;

use crate::{Id, Location, LocationRenderObj, Visual, VisualRenderObj};

#[derive(Debug, From)]
pub enum SceneChange {
    Matrix(Change<Matrix4>),
    Location(Change<LocationRenderObj>),
    Visual(Change<VisualRenderObj>),
}

impl SceneChange {
    pub fn destructive_change(&self) -> Option<(TypeId, Id)> {
        match self {
            SceneChange::Matrix(Change::Delete(id)) => Some((TypeId::of::<Matrix4>(), *id)),
            SceneChange::Visual(Change::Delete(id)) => Some((TypeId::of::<Visual>(), *id)),
            SceneChange::Location(Change::Delete(id)) => Some((TypeId::of::<Location>(), *id)),
            // .. to prevent missing new cases:
            SceneChange::Matrix(_) | SceneChange::Location(_) | SceneChange::Visual(_) => None,
        }
    }
}

#[derive(Debug)]
pub enum Change<T> {
    Create(Id, T),
    Delete(Id),
    Update(Id, T),
}
