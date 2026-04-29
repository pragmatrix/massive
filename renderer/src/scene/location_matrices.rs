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
    // The result of location property computation.
    location_properties: IdTable<Computed<ResolvedLocation>>,
    location_matrices: IdTable<Versioned<Matrix4>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ResolvedLocation {
    transform: Transform,
    alpha: f32,
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

        // Performance: `updated_at` covers both resolved transform and inherited alpha. An
        // alpha-only location change can therefore recompute this matrix even though the transform
        // is unchanged. Split transform/alpha versioning if this shows up in profiles.
        self.location_matrices
            .mut_or_default(location_id)
            .resolve(updated_at, || {
                self.location_properties[location_id].transform.to_matrix4()
            });
    }

    pub fn get_matrix(&self, location_id: Id) -> &Matrix4 {
        &self.location_matrices[location_id]
    }

    pub fn get_alpha(&self, location_id: Id) -> f32 {
        self.location_properties[location_id].alpha
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
    type Computed = ResolvedLocation;

    fn get_source(scene: &Scene, id: Id) -> &Versioned<Self::Source> {
        scene.locations.get_unwrapped(id)
    }

    fn resolve_dependencies(
        head_version: Version,
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
            .max(scene.transforms[transform_id].updated_at);

        // Combine with the optional parent location.
        if let Some(parent_id) = parent_id {
            // Make sure the parent is up to date and combine its max_deps_version.
            resolve::<Self>(head_version, scene, caches, parent_id).max(max_deps_version)
        } else {
            max_deps_version
        }
    }

    fn computed_mut(caches: &mut LocationTransforms, id: Id) -> &mut Computed<Self::Computed> {
        caches.location_properties.mut_or_default(id)
    }

    fn compute(
        scene: &Scene,
        caches: &LocationTransforms,
        source: &Self::Source,
    ) -> Self::Computed {
        let (parent_id, transform_id) = (source.parent, source.transform);
        let local_transform = &*scene.transforms[transform_id];
        let local_alpha = source.alpha;
        parent_id.map_or_else(
            || ResolvedLocation {
                transform: *local_transform,
                alpha: local_alpha,
            },
            |parent_id| {
                let parent = &caches.location_properties[parent_id];
                ResolvedLocation {
                    transform: parent.transform * *local_transform,
                    alpha: parent.alpha * local_alpha,
                }
            },
        )
    }
}

impl Default for Versioned<ResolvedLocation> {
    fn default() -> Self {
        Versioned::new(
            ResolvedLocation {
                transform: Transform::default(),
                alpha: 1.0,
            },
            0,
        )
    }
}

impl Default for Versioned<Transform> {
    fn default() -> Self {
        Versioned::new(Transform::default(), 0)
    }
}

#[cfg(test)]
mod tests {
    use massive_geometry::Vector3;
    use massive_scene::{Change, Location, SceneChange, id_generator};

    use super::*;
    use crate::TransactionManager;

    #[test]
    fn root_alpha_resolves_to_local_alpha() {
        let (scene, transaction, location_id) = scene_with_root_location(0.4);
        let mut locations = LocationTransforms::default();

        locations.resolve_locations_and_matrices(&scene, &transaction, [location_id].into_iter());

        assert_eq!(locations.get_alpha(location_id), 0.4);
    }

    #[test]
    fn child_alpha_multiplies_parent_alpha() {
        let mut transaction_manager = TransactionManager::default();
        let mut scene = Scene::default();
        let parent_transform_id = new_transform_id();
        let child_transform_id = new_transform_id();
        let parent_location_id = new_location_id();
        let child_location_id = new_location_id();
        let transaction = transaction_manager.new_transaction();

        scene.apply(
            &SceneChange::Transform(Change::Create(parent_transform_id, Transform::IDENTITY)),
            &transaction,
        );
        scene.apply(
            &SceneChange::Transform(Change::Create(child_transform_id, Transform::IDENTITY)),
            &transaction,
        );
        scene.apply(
            &SceneChange::Location(Change::Create(
                parent_location_id,
                LocationRenderObj {
                    parent: None,
                    transform: parent_transform_id,
                    alpha: 0.5,
                },
            )),
            &transaction,
        );
        scene.apply(
            &SceneChange::Location(Change::Create(
                child_location_id,
                LocationRenderObj {
                    parent: Some(parent_location_id),
                    transform: child_transform_id,
                    alpha: 0.25,
                },
            )),
            &transaction,
        );

        let mut locations = LocationTransforms::default();
        locations.resolve_locations_and_matrices(
            &scene,
            &transaction,
            [child_location_id].into_iter(),
        );

        assert_eq!(locations.get_alpha(child_location_id), 0.125);
    }

    #[test]
    fn transform_and_alpha_changes_invalidate_resolved_location() {
        let mut transaction_manager = TransactionManager::default();
        let mut scene = Scene::default();
        let transform_id = new_transform_id();
        let location_id = new_location_id();
        let initial_transform = Transform::from_translation(Vector3::new(1.0, 2.0, 0.0));
        let moved_transform = Transform::from_translation(Vector3::new(7.0, 11.0, 0.0));
        let transaction = transaction_manager.new_transaction();

        scene.apply(
            &SceneChange::Transform(Change::Create(transform_id, initial_transform)),
            &transaction,
        );
        scene.apply(
            &SceneChange::Location(Change::Create(
                location_id,
                LocationRenderObj {
                    parent: None,
                    transform: transform_id,
                    alpha: 0.25,
                },
            )),
            &transaction,
        );

        let mut locations = LocationTransforms::default();
        locations.resolve_locations_and_matrices(&scene, &transaction, [location_id].into_iter());
        let initial_matrix = *locations.get_matrix(location_id);
        assert_eq!(locations.get_alpha(location_id), 0.25);

        let transaction = transaction_manager.new_transaction();
        scene.apply(
            &SceneChange::Transform(Change::Update(transform_id, moved_transform)),
            &transaction,
        );
        locations.resolve_locations_and_matrices(&scene, &transaction, [location_id].into_iter());
        assert_ne!(*locations.get_matrix(location_id), initial_matrix);
        assert_eq!(locations.get_alpha(location_id), 0.25);

        let transaction = transaction_manager.new_transaction();
        scene.apply(
            &SceneChange::Location(Change::Update(
                location_id,
                LocationRenderObj {
                    parent: None,
                    transform: transform_id,
                    alpha: 0.75,
                },
            )),
            &transaction,
        );
        locations.resolve_locations_and_matrices(&scene, &transaction, [location_id].into_iter());
        assert_eq!(locations.get_alpha(location_id), 0.75);
    }

    #[test]
    fn default_resolved_location_is_opaque() {
        let default = Versioned::<ResolvedLocation>::default();

        assert_eq!(default.alpha, 1.0);
    }

    fn scene_with_root_location(alpha: f32) -> (Scene, Transaction, Id) {
        let mut transaction_manager = TransactionManager::default();
        let mut scene = Scene::default();
        let transform_id = new_transform_id();
        let location_id = new_location_id();
        let transaction = transaction_manager.new_transaction();

        scene.apply(
            &SceneChange::Transform(Change::Create(transform_id, Transform::IDENTITY)),
            &transaction,
        );
        scene.apply(
            &SceneChange::Location(Change::Create(
                location_id,
                LocationRenderObj {
                    parent: None,
                    transform: transform_id,
                    alpha,
                },
            )),
            &transaction,
        );

        (scene, transaction, location_id)
    }

    fn new_transform_id() -> Id {
        id_generator::acquire::<Transform>()
    }

    fn new_location_id() -> Id {
        id_generator::acquire::<Location>()
    }
}
