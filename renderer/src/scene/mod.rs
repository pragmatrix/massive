use massive_geometry::Matrix4;
use massive_scene::{Change, Id, LocationRenderObj, SceneChange, VisualRenderObj};
use versioning::Versioned;

use crate::{Transaction, Version};

mod dependency_resolver;
mod id_table;
mod versioning;
mod location_matrices;

pub use id_table::IdTable;
pub use location_matrices::LocationMatrices;

#[derive(Debug, Default)]
pub struct Scene {
    // Option: Because setting the values to None deletes then.
    //
    // Optimization: Defaults could be used here, too, but Matrix4 currently does not define a
    // default(), and using defaults has the drawback, that referential errors may lead to confusing
    // render results instead of a panic.
    matrices: IdTable<Option<Versioned<Matrix4>>>,
    locations: IdTable<Option<Versioned<LocationRenderObj>>>,
    // This is also versioned to allow the renderer to cache derived stuff.
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
            SceneChange::Matrix(change) => self.matrices.apply_versioned(change, current_version),
            SceneChange::Location(change) => {
                self.locations.apply_versioned(change, current_version)
            }
            SceneChange::Visual(change) => self.visuals.apply(change),
        }
    }

    // Returns a set of grouped shape by matrix.
    // pub fn grouped_shapes(
    //     &self,
    //     transaction: &Transaction,
    // ) -> impl Iterator<Item = (Matrix4, impl Iterator<Item = &Shape> + Clone)> {
    //     let mut map: HashMap<Id, Vec<&[Shape]>> = HashMap::new();

    //     for visual in self.visuals.iter_some() {
    //         let location_id = visual.location;
    //         map.entry(location_id).or_default().push(&visual.shapes);
    //     }

    //     // Update all matrices that are in use.
    //     {
    //         let version = transaction.current_version();
    //         let mut caches = self.caches.borrow_mut();
    //         for location_id in map.keys() {
    //             self.resolve_visual_matrix(*location_id, version, &mut caches);
    //         }
    //     }

    //     // Create the group iterator.

    //     let caches = self.caches.borrow();

    //     map.into_iter().map(move |(visual_id, shapes)| {
    //         // We can't return a reference to matrix, because this would also borrow `caches`.
    //         let matrix = *caches.location_matrix[visual_id];
    //         (matrix, shapes.into_iter().flatten())
    //     })
    // }
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

impl<T> IdTable<Option<T>> {
    /// Iterate through all existing (non-`None`) values.
    pub fn iter_some(&self) -> impl Iterator<Item = &T> {
        self.iter().filter_map(|v| v.as_ref())
    }

    pub fn apply(&mut self, change: Change<T>) {
        match change {
            Change::Create(id, value) => self.insert(id, Some(value)),
            Change::Delete(id) => self[id] = None,
            Change::Update(id, value) => {
                // A value at this index must exist, so use `rows_mut()` here.
                self[id] = Some(value)
            }
        }
    }

    /// Returns a reference to the object at `id`.
    ///
    /// Panics if it does not exist.
    pub fn get_unwrapped(&self, id: Id) -> &T {
        self[id].as_ref().unwrap()
    }
}
