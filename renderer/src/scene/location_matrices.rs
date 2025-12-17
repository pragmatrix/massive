use massive_geometry::Matrix4;
use massive_scene::{Id, LocationRenderObj, Transform};

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
pub struct LocationTransforms {
    // The result of a location matrix computation.
    location_transforms: IdTable<Computed<Transform>>,
    location_matrices: IdTable<Versioned<Matrix4>>,
}

impl LocationTransforms {
    pub fn resolve_locations_and_matrices(
        &mut self,
        scene: &Scene,
        transaction: &Transaction,
        locations: impl Iterator<Item = Id>,
    ) {
        locations.for_each(|id| {
            self.resolve_location_and_matrix(scene, id, transaction);
        });
    }

    /// Compute - if needed - the location and its final matrix.
    ///
    /// When this function returns the matrix at `location_id` is up to date with the current
    /// version and can be used for rendering.
    fn resolve_location_and_matrix(
        &mut self,
        scene: &Scene,
        location_id: Id,
        current_version: &Transaction,
    ) {
        let updated_at =
            resolve::<VisualLocation>(current_version.current_version(), scene, self, location_id);

        self.location_matrices
            .mut_or_default(location_id)
            .resolve(updated_at, || {
                self.location_transforms[location_id].to_matrix4()
            });
    }

    pub fn get_matrix(&self, location_id: Id) -> &Matrix4 {
        &self.location_matrices[location_id]
    }
}

// Quick hack to prevent the use of Option<Versioned>
impl Default for Versioned<Matrix4> {
    fn default() -> Self {
        Self::new(Matrix4::ZERO, 0)
    }
}

/// The dependency resolver for final location of a [`Visual`].
struct VisualLocation;

impl DependencyResolver for VisualLocation {
    type SourceStorage = Scene;
    type Source = LocationRenderObj;

    type ComputedStorage = LocationTransforms;
    type Computed = Transform;

    fn get_source(scene: &Scene, id: Id) -> &Versioned<Self::Source> {
        scene.locations.get_unwrapped(id)
    }

    fn resolve_dependencies(
        current_version: Version,
        source: &Versioned<Self::Source>,
        scene: &Scene,
        caches: &mut LocationTransforms,
    ) -> Version {
        let (parent_id, transform_id) = (source.parent, source.transform);

        // Find out the max version of all the immediate and (indirect / computed) dependencies.

        // Get the _three_ versions of the elements this one is computed on.
        // a) The self location's version.
        // b) The local transform's version.
        // c) The computed transform of the parent (representing all its dependencies).
        let max_deps_version = source
            .updated_at
            .max(scene.transforms.get_unwrapped(transform_id).updated_at);

        // Combine with the optional parent location.
        if let Some(parent_id) = parent_id {
            // Make sure the parent is up to date and combine its max_deps_version.
            resolve::<Self>(current_version, scene, caches, parent_id).max(max_deps_version)
        } else {
            max_deps_version
        }
    }

    fn computed_mut(caches: &mut LocationTransforms, id: Id) -> &mut Computed<Self::Computed> {
        caches.location_transforms.mut_or_default(id)
    }

    fn compute(
        scene: &Scene,
        caches: &LocationTransforms,
        source: &Self::Source,
    ) -> Self::Computed {
        let (parent_id, transform_id) = (source.parent, source.transform);
        let local_transform = &**scene.transforms.get_unwrapped(transform_id);
        parent_id.map_or_else(
            || *local_transform,
            |parent_id| *caches.location_transforms[parent_id] * *local_transform,
        )
    }
}
