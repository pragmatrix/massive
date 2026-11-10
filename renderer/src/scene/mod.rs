use massive_scene::{Id, Location, RenderChange, RenderSceneChange, Transform, Visual};

use crate::{Transaction, Version, tools::Versioned};

mod dependency_resolver;
mod id_table;
mod location_matrices;
mod scene_state;

pub use id_table::IdTable;
pub use location_matrices::LocationTransforms;
pub use scene_state::SceneState;

#[derive(Debug, Default)]
pub struct Scene {
    // Option: Because setting the values to None deletes then.
    //
    // Optimization: Defaults could be used here.
    transforms: IdTable<Versioned<Transform>>,
    locations: IdTable<Option<Versioned<Location>>>,
    visuals: IdTable<Option<Visual>>,
}

impl Scene {
    /// Integrate one scene change into the scene.
    ///
    /// The transaction is given a new version number, which is then treated as the most recent
    /// version and the current version of the whole scene.
    pub fn apply(&mut self, change: RenderSceneChange, transaction: &Transaction) {
        let current_version = transaction.current_version();
        match change {
            RenderSceneChange::Transform(change) => {
                self.transforms.apply_versioned(change, current_version)
            }
            RenderSceneChange::Location(change) => {
                self.locations.apply_versioned(change, current_version)
            }
            RenderSceneChange::Visual(change) => self.visuals.apply(change),
        }
    }

    pub fn visuals(&self) -> &IdTable<Option<Visual>> {
        &self.visuals
    }
}

impl<T> IdTable<Option<Versioned<T>>> {
    pub fn apply_versioned(&mut self, change: RenderChange<T>, version: Version) {
        match change {
            RenderChange::Create(id, value) => {
                self.insert(id, Some(Versioned::new(value, version)))
            }
            RenderChange::Delete(id) => self[id] = None,
            RenderChange::Update(id, value) => self[id] = Some(Versioned::new(value, version)),
        }
    }
}

impl<T> IdTable<Versioned<T>>
where
    Versioned<T>: Default,
{
    pub fn apply_versioned(&mut self, change: RenderChange<T>, version: Version) {
        match change {
            RenderChange::Create(id, value) => self.insert(id, Versioned::new(value, version)),
            RenderChange::Delete(id) => self[id] = Versioned::default(),
            RenderChange::Update(id, value) => self[id] = Versioned::new(value, version),
        }
    }
}

impl<T> IdTable<Option<T>> {
    pub fn apply(&mut self, change: RenderChange<T>) {
        match change {
            RenderChange::Create(id, value) => self.insert(id, Some(value)),
            RenderChange::Delete(id) => self[id] = None,
            RenderChange::Update(id, value) => self[id] = Some(value),
        }
    }

    /// Returns a reference to the object at `id`.
    ///
    /// Panics if it does not exist.
    pub fn get_unwrapped(&self, id: Id) -> &T {
        self[id].as_ref().unwrap()
    }
}
