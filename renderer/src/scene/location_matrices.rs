use euclid::num::Zero;
use massive_geometry::Matrix4;
use massive_scene::{Id, LocationRenderObj};

use crate::{
    Transaction, Version,
    scene::{
        IdTable, Scene,
        dependency_resolver::{DependencyResolver, resolve},
    },
    tools::{Computed, Versioned},
};

/// Computed matrices of all the visuals.
#[derive(Debug, Default)]
pub struct LocationMatrices {
    // The result of a location matrix computation.
    location_matrix: IdTable<Computed<Matrix4>>,
}

impl LocationMatrices {
    pub fn compute_matrices(
        &mut self,
        scene: &Scene,
        transaction: &Transaction,
        locations: impl Iterator<Item = Id>,
    ) {
        locations.for_each(|id| {
            self.compute_visual_matrix(scene, id, transaction);
        });
    }

    /// Returns a reference to a matrix of the location.
    pub fn get(&self, location_id: Id) -> &Matrix4 {
        &self.location_matrix[location_id]
    }

    /// Compute - if needed - the matrix of a location.
    ///
    /// When this function returns the matrix at `location_id` is up to date with the current
    /// version and can be used for rendering.
    fn compute_visual_matrix(
        &mut self,
        scene: &Scene,
        location_id: Id,
        current_version: &Transaction,
    ) {
        resolve::<VisualMatrix>(current_version.current_version(), scene, self, location_id);
    }
}

/// The dependency resolver for final matrix of a [`Visual`].
struct VisualMatrix;

impl DependencyResolver for VisualMatrix {
    type SharedStorage = Scene;
    type ComputedStorage = LocationMatrices;
    type Source = LocationRenderObj;
    type Computed = Matrix4;

    fn source(scene: &Scene, id: Id) -> &Versioned<Self::Source> {
        scene.locations.get_unwrapped(id)
    }

    fn resolve_dependencies(
        current_version: Version,
        source: &Versioned<Self::Source>,
        scene: &Scene,
        caches: &mut LocationMatrices,
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

        // Combine with the optional parent location.
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

    fn computed_mut(caches: &mut LocationMatrices, id: Id) -> &mut Computed<Self::Computed> {
        caches.location_matrix.mut_or_default(id)
    }

    fn compute(scene: &Scene, caches: &LocationMatrices, source: &Self::Source) -> Self::Computed {
        let (parent_id, matrix_id) = (source.parent, source.matrix);
        let local_matrix = &**scene.matrices.get_unwrapped(matrix_id);
        parent_id.map_or_else(
            || *local_matrix,
            |parent_id| *caches.location_matrix[parent_id] * local_matrix,
        )
    }
}

/// This is here to avoid using `Option` in the computed IdTable.
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
