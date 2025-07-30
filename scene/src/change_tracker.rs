use std::{any::TypeId, mem, ops::DerefMut, sync::Mutex};

use derive_more::From;

use crate::{Id, Location, LocationRenderObj, Visual, VisualRenderObj};
use massive_geometry as geometry;

#[derive(Debug, Default)]
pub struct ChangeTracker(Mutex<Vec<SceneChange>>);

impl ChangeTracker {
    pub fn push(&self, change: impl Into<SceneChange>) {
        self.0.lock().unwrap().push(change.into());
    }

    pub(crate) fn take_all(&self) -> Vec<SceneChange> {
        mem::take(self.0.lock().unwrap().deref_mut())
    }
}

#[derive(Debug, From)]
pub enum SceneChange {
    Matrix(Change<geometry::Matrix4>),
    Location(Change<LocationRenderObj>),
    Visual(Change<VisualRenderObj>),
}

impl SceneChange {
    pub fn destructive_change(&self) -> Option<(TypeId, Id)> {
        match self {
            SceneChange::Matrix(Change::Delete(id)) => {
                Some((TypeId::of::<geometry::Matrix4>(), *id))
            }
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
