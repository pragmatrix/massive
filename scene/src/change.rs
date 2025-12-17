use std::any::TypeId;

use derive_more::From;
use massive_geometry::Transform;

use crate::{Id, Location, LocationRenderObj, Visual, VisualRenderObj};

#[derive(Debug, From, Clone)]
pub enum SceneChange {
    Transform(Change<Transform>),
    Location(Change<LocationRenderObj>),
    Visual(Change<VisualRenderObj>),
}

impl SceneChange {
    pub fn destructive_change(&self) -> Option<(TypeId, Id)> {
        match self {
            SceneChange::Transform(Change::Delete(id)) => Some((TypeId::of::<Transform>(), *id)),
            SceneChange::Visual(Change::Delete(id)) => Some((TypeId::of::<Visual>(), *id)),
            SceneChange::Location(Change::Delete(id)) => Some((TypeId::of::<Location>(), *id)),
            // .. match exhaustive.
            SceneChange::Transform(_) | SceneChange::Location(_) | SceneChange::Visual(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Change<T> {
    Create(Id, T),
    Update(Id, T),
    Delete(Id),
}

impl<T> Change<T> {
    pub fn id(&self) -> Id {
        match *self {
            Change::Create(id, _) => id,
            Change::Update(id, _) => id,
            Change::Delete(id) => id,
        }
    }
}
