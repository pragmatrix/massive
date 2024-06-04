use std::{any::TypeId, mem};

use massive_geometry as geometry;

use crate::{Id, Object, PositionedRenderShape, PositionedShape};

#[derive(Debug)]
pub enum Change<T> {
    Create(Id, T),
    Delete(Id),
    Update(Id, T),
}

#[derive(Debug)]
pub enum SceneChange {
    Matrix(Change<geometry::Matrix4>),
    PositionedShape(Change<PositionedRenderShape>),
}

impl SceneChange {
    pub fn destructive_change(&self) -> Option<(TypeId, Id)> {
        match self {
            SceneChange::Matrix(Change::Delete(id)) => {
                Some((TypeId::of::<geometry::Matrix4>(), *id))
            }
            SceneChange::PositionedShape(Change::Delete(id)) => {
                Some((TypeId::of::<PositionedShape>(), *id))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ChangeTracker(Vec<SceneChange>);

impl ChangeTracker {
    pub fn create<T: Object>(&mut self, id: Id, value: T::Uploaded) {
        self.push::<T>(Change::Create(id, value))
    }

    pub fn update<T: Object>(&mut self, id: Id, value: T::Uploaded) {
        self.push::<T>(Change::Update(id, value))
    }

    pub fn delete<T: Object>(&mut self, id: Id) {
        self.push::<T>(Change::Delete(id))
    }

    fn push<T: Object>(&mut self, change: Change<T::Uploaded>) {
        self.0.push(T::promote_change(change));
    }

    pub(crate) fn take_all(&mut self) -> Vec<SceneChange> {
        mem::take(&mut self.0)
    }
}
