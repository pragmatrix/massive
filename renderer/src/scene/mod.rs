use massive_scene::{Change, Id, LocationRenderObj, SceneChange, Transform, VisualRenderObj};

use crate::{Transaction, Version, tools::Versioned};

mod dependency_resolver;
mod id_table;
mod location_matrices;

pub use id_table::IdTable;
pub use location_matrices::LocationTransforms;

#[derive(Debug, Default)]
pub struct Scene {
    // Option: Because setting the values to None deletes then.
    //
    // Optimization: Defaults could be used here.
    transforms: IdTable<Versioned<Transform>>,
    locations: IdTable<Option<Versioned<LocationRenderObj>>>,
    visuals: IdTable<Option<VisualRenderObj>>,
}

impl Scene {
    /// Integrate one scene change into the scene.
    ///
    /// The transaction is given a new version number, which is then treated as the most recent
    /// version and the current version of the whole scene.
    pub fn apply(&mut self, change: &SceneChange, transaction: &Transaction) {
        let current_version = transaction.current_version();
        match change.clone() {
            SceneChange::Transform(change) => {
                self.transforms.apply_versioned(change, current_version)
            }
            SceneChange::Location(change) => {
                self.locations.apply_versioned(change, current_version)
            }
            SceneChange::Visual(change) => self.visuals.apply(change),
        }
    }

    pub fn visuals(&self) -> &IdTable<Option<VisualRenderObj>> {
        &self.visuals
    }
}

impl<T> IdTable<Option<Versioned<T>>> {
    pub fn apply_versioned(&mut self, change: Change<T>, version: Version) {
        match change {
            Change::Create(id, value) => self.insert(id, Some(Versioned::new(value, version))),
            Change::Delete(id) => self[id] = None,
            Change::Update(id, value) => self[id] = Some(Versioned::new(value, version)),
        }
    }
}

impl<T> IdTable<Versioned<T>>
where
    Versioned<T>: Default,
{
    pub fn apply_versioned(&mut self, change: Change<T>, version: Version) {
        match change {
            Change::Create(id, value) => self.insert(id, Versioned::new(value, version)),
            Change::Delete(id) => self[id] = Versioned::default(),
            Change::Update(id, value) => self[id] = Versioned::new(value, version),
        }
    }
}

impl<T> IdTable<Option<T>> {
    pub fn apply(&mut self, change: Change<T>) {
        match change {
            Change::Create(id, value) => self.insert(id, Some(value)),
            Change::Delete(id) => self[id] = None,
            Change::Update(id, value) => self[id] = Some(value),
        }
    }

    /// Returns a reference to the object at `id`.
    ///
    /// Panics if it does not exist.
    pub fn get_unwrapped(&self, id: Id) -> &T {
        self[id].as_ref().unwrap()
    }
}
