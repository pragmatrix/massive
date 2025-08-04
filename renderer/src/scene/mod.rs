use std::{cell::RefCell, collections::HashMap};

use euclid::num::Zero;

use dependency_resolver::{resolve, DependencyResolver};
use id_table::IdTable;
use massive_geometry::Matrix4;
use massive_scene::{Change, Id, LocationRenderObj, SceneChange, Shape, VisualRenderObj};
use versioning::{Computed, Versioned};

use crate::{Transaction, Version};

mod dependency_resolver;
mod id_table;
mod versioning;

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

    caches: RefCell<SceneCaches>,
}

impl Scene {
    /// Integrate one scene change into the scene.
    ///
    /// The transaction is given a new version number, which is then treated as the most recent
    /// version and the current version of the whole scene.
    pub fn apply(&mut self, change: SceneChange, transaction: &Transaction) {
        let current_version = transaction.current_version();
        match change {
            SceneChange::Matrix(change) => self.matrices.apply_versioned(change, current_version),
            SceneChange::Location(change) => {
                self.locations.apply_versioned(change, current_version)
            }
            SceneChange::Visual(change) => self.visuals.apply(change),
        }
    }

    /// Returns a set of grouped shape by matrix.
    pub fn grouped_shapes(
        &self,
        transaction: &Transaction,
    ) -> impl Iterator<Item = (Matrix4, impl Iterator<Item = &Shape> + Clone)> {
        let mut map: HashMap<Id, Vec<&[Shape]>> = HashMap::new();

        for visual in self.visuals.iter_some() {
            let location_id = visual.location;
            map.entry(location_id).or_default().push(&visual.shapes);
        }

        // Update all matrices that are in use.
        {
            let version = transaction.current_version();
            let mut caches = self.caches.borrow_mut();
            for visual_id in map.keys() {
                self.resolve_visual_matrix(*visual_id, version, &mut caches);
            }
        }

        // Create the group iterator.

        let caches = self.caches.borrow();

        map.into_iter().map(move |(visual_id, shapes)| {
            // We can't return a reference to matrix, because this would also borrow `caches`.
            let matrix = *caches.location_matrix[visual_id];
            (matrix, shapes.into_iter().flatten())
        })
    }

    /// Compute - if needed - the matrix of a location.
    ///
    /// When this function returns the matrix at `location_id` is up to date with the current
    /// version and can be used for rendering.
    fn resolve_visual_matrix(
        &self,
        location_id: Id,
        current_version: Version,
        caches: &mut SceneCaches,
    ) {
        resolve::<VisualMatrix>(current_version, self, caches, location_id);
    }
}

/// The dependency resolver for final matrix of a [`Visual`].
struct VisualMatrix;

impl DependencyResolver for VisualMatrix {
    type SharedStorage = Scene;
    type ComputedStorage = SceneCaches;
    type Source = LocationRenderObj;
    type Computed = Matrix4;

    fn source(scene: &Scene, id: Id) -> &Versioned<Self::Source> {
        scene.locations.get_unwrapped(id)
    }

    fn resolve_dependencies(
        current_version: Version,
        source: &Versioned<Self::Source>,
        scene: &Scene,
        caches: &mut SceneCaches,
    ) -> Version {
        let (parent_id, matrix_id) = (source.parent, source.matrix);

        // Find out the max version of all the immediate and (indirect / computed) dependencies.

        // Get the _three_ versions of the elements this one is computed on.
        // a) The self location's version.
        // b) The local matrix's version.
        // c) The computed matrix of the parent (representing all its dependencies).
        let max_deps_version = source
            .updated_at
            .max(scene.matrices.get_unwrapped(matrix_id).updated_at);

        // Combine with the optional parent.
        if let Some(parent_id) = parent_id {
            // Make sure the parent is up to date.
            resolve::<Self>(current_version, scene, caches, parent_id);
            caches.location_matrix[parent_id]
                .max_deps_version
                .max(max_deps_version)
        } else {
            max_deps_version
        }
    }

    fn computed_mut(caches: &mut SceneCaches, id: Id) -> &mut Computed<Self::Computed> {
        caches.location_matrix.mut_or_default(id)
    }

    fn compute(scene: &Scene, caches: &SceneCaches, source: &Self::Source) -> Self::Computed {
        let (parent_id, matrix_id) = (source.parent, source.matrix);
        let local_matrix = &**scene.matrices.get_unwrapped(matrix_id);
        parent_id.map_or_else(
            || *local_matrix,
            |parent_id| *caches.location_matrix[parent_id] * local_matrix,
        )
    }
}

impl<T> IdTable<Option<T>> {
    /// Iterate through all existing (non-`None`) values.
    pub fn iter_some(&self) -> impl Iterator<Item = &T> {
        self.iter().filter_map(|v| v.as_ref())
    }

    pub fn apply(&mut self, change: Change<T>) {
        match change {
            Change::Create(id, value) => self.put(id, Some(value)),
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

impl<T> IdTable<Option<Versioned<T>>> {
    pub fn apply_versioned(&mut self, change: Change<T>, version: Version) {
        match change {
            Change::Create(id, value) => self.put(id, Some(Versioned::new(value, version))),
            Change::Delete(id) => self[id] = None,
            Change::Update(id, value) => self[id] = Some(Versioned::new(value, version)),
        }
    }
}

impl Default for Computed<Matrix4> {
    fn default() -> Self {
        Self {
            validated_at: 0,
            max_deps_version: 0,
            // OO: is there a way to use `::ZERO` / the trait `ConstZero` from num_traits for
            // example?
            value: Matrix4::zero(),
        }
    }
}

#[derive(Debug, Default)]
struct SceneCaches {
    // The result of a location matrix computation.
    location_matrix: IdTable<Computed<Matrix4>>,
}
