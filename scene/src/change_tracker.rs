use massive_geometry as geometry;

use crate::{Id, Object};

#[derive(Debug)]
pub enum Change<T> {
    Create(Id, T),
    Delete(Id),
    Update(Id, T),
}

#[derive(Debug)]
pub enum SceneChange {
    Matrix(Change<geometry::Matrix4>),
}

#[derive(Debug, Default)]
pub struct ChangeTracker(Vec<SceneChange>);

impl ChangeTracker {
    pub fn push<T: Object>(&mut self, change: Change<T>) {
        self.0.push(T::promote_change(change))
    }
}
