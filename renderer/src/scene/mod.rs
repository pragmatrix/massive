use std::{cell::RefCell, collections::HashMap};

use id_table::IdTable;
use massive_geometry::Matrix4;
use massive_scene::{Change, Id, PositionRenderObj, PositionedRenderShape, SceneChange, Shape};
use versioning::{Computed, Version, Versioned};

mod id_table;
mod versioning;

#[derive(Debug, Default)]
pub struct Scene {
    /// The version of newest values in the tables.
    current_version: Version,

    matrices: IdTable<Option<Versioned<Matrix4>>>,
    positions: IdTable<Option<Versioned<PositionRenderObj>>>,
    shapes: IdTable<Option<PositionedRenderShape>>,

    caches: RefCell<SceneCaches>,
}

impl Scene {
    /// Integrate a number of scene changes as single transaction into the scene.
    ///
    /// The transaction is given a new version number, which is then treated as the most recent
    /// version and the current version of the whole scene.
    pub fn transact(&mut self, changes: impl IntoIterator<Item = SceneChange>) {
        self.current_version += 1;
        for change in changes {
            self.apply(change, self.current_version)
        }
    }

    fn apply(&mut self, change: SceneChange, version: Version) {
        match change {
            SceneChange::Matrix(change) => self.matrices.apply_versioned(change, version),
            SceneChange::Position(change) => self.positions.apply_versioned(change, version),
            SceneChange::PositionedShape(change) => self.shapes.apply(change),
        }
    }

    /// Returns a set of grouped shape by matrix.
    ///
    /// TODO: This should not be &mut self, because it updates computed values only.
    pub fn grouped_shapes(&self) -> impl Iterator<Item = (Matrix4, Vec<&Shape>)> {
        let mut map: HashMap<Id, Vec<&Shape>> = HashMap::new();

        for positioned in self.shapes.iter_some() {
            let position_id = positioned.position;
            map.entry(position_id).or_default().push(&positioned.shape);
        }

        // Update all matrices that are in use.
        {
            let mut caches = self.caches.borrow_mut();
            for position_id in map.keys() {
                self.resolve_positioned_matrix(*position_id, &mut caches);
            }
        }

        // Create the group iterator.

        let caches = self.caches.borrow();

        map.into_iter().map(move |(position_id, shapes)| {
            // Ensure the matrix is up2date.
            // We can't return a reference to matrix, because this would also borrow `caches``.
            let matrix = *caches.positions_matrix[position_id];
            (matrix, shapes)
        })
    }

    /// Compute - if needed - the matrix of a position.
    ///
    /// When this function returns the matrix at `position_id` is up to date with the current
    /// version and can be used for rendering.
    ///
    /// We don't return a reference to the result here, because the borrow checker would make this
    /// recursive function invocation uncessarily more complex.
    ///
    /// TODO: Unrecurse this. There might be degenerate cases of large dependency chains.
    fn resolve_positioned_matrix(&self, position_id: Id, caches: &mut SceneCaches) {
        let current_version = self.current_version;
        // Already validated at the latest version? Done.
        if caches.positions_matrix[position_id].validated_at == current_version {
            return;
        }

        let position = &self.positions[position_id].as_ref().unwrap();
        let (parent_id, matrix) = (position.parent, position.matrix);

        // Find out the max version of all the immeidate and (indirect / computed) dependencies.

        // Get the _three_ versions of the elements this one is computed on.
        // a) The self position's version.
        // b) The local matrix's version.
        // c) The computed matrix of the parent (representing all its dependencies).
        let max_deps_version = position
            .updated_at
            .max(self.matrices[matrix].as_ref().unwrap().updated_at);

        // Combine with the optional parent.
        let max_deps_version = {
            if let Some(parent_id) = parent_id {
                // Be sure the parent is up to date.
                self.resolve_positioned_matrix(parent_id, caches);
                caches.positions_matrix[parent_id]
                    .max_deps_version
                    .max(max_deps_version)
            } else {
                max_deps_version
            }
        };

        // If the max_deps_version is smaller or equal to the current one, the value is ok and can
        // be marked as validated for this round.
        {
            let positioned_matrix = &mut caches.positions_matrix[position_id];
            if max_deps_version <= positioned_matrix.max_deps_version {
                positioned_matrix.validated_at = current_version;
                return;
            }
        }

        // Compute a new value.

        let local_matrix = &**self.matrices[matrix].as_ref().unwrap();
        let new_value = parent_id.map_or_else(
            || *local_matrix,
            |parent_id| *caches.positions_matrix[parent_id] * local_matrix,
        );

        caches.positions_matrix[position_id] = Computed {
            validated_at: current_version,
            max_deps_version,
            value: new_value,
        };
    }
}

impl<T> IdTable<Option<T>> {
    pub fn iter_some(&self) -> impl Iterator<Item = &T> {
        self.iter().filter_map(|v| v.as_ref())
    }

    pub fn apply(&mut self, change: Change<T>) {
        match change {
            Change::Create(id, value) => self.put(id, Some(value)),
            Change::Delete(id) => self.put(id, None),
            Change::Update(id, value) => {
                // Already know that this index must exist, so use rows() here.
                self.rows_mut()[*id] = Some(value)
            }
        }
    }
}

impl<T> IdTable<Option<Versioned<T>>> {
    pub fn apply_versioned(&mut self, change: Change<T>, version: Version) {
        match change {
            Change::Create(id, value) => self.put(id, Some(Versioned::new(value, version))),
            Change::Delete(id) => self.put(id, None),
            Change::Update(id, value) => {
                self.rows_mut()[*id] = Some(Versioned::new(value, version))
            }
        }
    }
}

#[derive(Debug, Default)]
struct SceneCaches {
    // The result of a positioned computation.
    positions_matrix: IdTable<Computed<Matrix4>>,
}
