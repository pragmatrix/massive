use std::{cell::RefCell, collections::HashMap};

use euclid::num::Zero;
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
    pub fn grouped_shapes(
        &self,
    ) -> impl Iterator<Item = (Matrix4, impl Iterator<Item = &Shape> + Clone)> {
        let mut map: HashMap<Id, Vec<&[Shape]>> = HashMap::new();

        for positioned in self.shapes.iter_some() {
            let position_id = positioned.position;
            map.entry(position_id).or_default().push(&positioned.shapes);
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
            // We can't return a reference to matrix, because this would also borrow `caches`.
            let matrix = *caches.positions_matrix[position_id];
            (matrix, shapes.into_iter().flatten())
        })
    }

    /// Compute - if needed - the matrix of a position.
    ///
    /// When this function returns the matrix at `position_id` is up to date with the current
    /// version and can be used for rendering.
    ///
    /// We don't return a reference to the result here, because the borrow checker would make this
    /// recursive function invocation unnecessarily more complex.
    ///
    /// TODO: Unrecurse this. There might be degenerate cases of large dependency chains.
    fn resolve_positioned_matrix(&self, position_id: Id, caches: &mut SceneCaches) {
        let current_version = self.current_version;
        // Already validated at the latest version? Done.
        //
        // `get_or_default` must be used here. This is the only situation in which the cache may
        // need to be resized.
        if caches
            .positions_matrix
            .mut_or_default(position_id)
            .validated_at
            == current_version
        {
            return;
        }

        let position = self.positions.get_unwrapped(position_id);
        let (parent_id, matrix_id) = (position.parent, position.matrix);

        // Find out the max version of all the immediate and (indirect / computed) dependencies.

        // Get the _three_ versions of the elements this one is computed on.
        // a) The self position's version.
        // b) The local matrix's version.
        // c) The computed matrix of the parent (representing all its dependencies).
        let max_deps_version = position
            .updated_at
            .max(self.matrices.get_unwrapped(matrix_id).updated_at);

        // Combine with the optional parent.
        let max_deps_version = {
            if let Some(parent_id) = parent_id {
                // Make sure the parent is up to date.
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

        let local_matrix = &**self.matrices.get_unwrapped(matrix_id);
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

fn resolve<Resolver: DependencyResolver>(scene: &Scene, caches: &mut SceneCaches, id: Id)
where
    Computed<Resolver::Computed>: Default,
{
    let current_version = scene.current_version;

    // Already validated at the latest version? Done.
    //
    // `get_or_default` must be used here. This is the only situation in which the cache may
    // need to be resized.
    let computed = Resolver::computed_mut(caches, id);
    if computed.validated_at == current_version {
        return;
    }
    // Save for later.
    let computed_max_deps = computed.max_deps_version;

    let source = Resolver::source(scene, id);
    let max_deps_version = Resolver::resolve_dependencies(scene, caches, source);

    // If the max_deps_version is smaller or equal to the current one, the value is ok and can
    // be marked as validated for this round.
    if max_deps_version <= computed_max_deps {
        Resolver::computed_mut(caches, id).validated_at = current_version;
        return;
    }

    // Compute a new value and store it.
    let new_value = Resolver::compute(scene, caches, source);
    *Resolver::computed_mut(caches, id) = Computed {
        validated_at: current_version,
        max_deps_version,
        value: new_value,
    };
}

trait DependencyResolver {
    /// Currently, this is used in the resolve algorithm for caching only.
    type Source;
    /// The computed value type (must implement Default for now, use Option<> otherwise)
    type Computed;

    fn source(scene: &Scene, id: Id) -> &Versioned<Self::Source>;

    /// Make sure that all dependencies are up to date and return their maximum version.
    fn resolve_dependencies(
        scene: &Scene,
        caches: &mut SceneCaches,
        source: &Versioned<Self::Source>,
    ) -> Version;

    fn compute(scene: &Scene, caches: &mut SceneCaches, source: &Self::Source) -> Self::Computed;

    fn computed_mut(caches: &mut SceneCaches, id: Id) -> &mut Computed<Self::Computed>
    where
        Computed<Self::Computed>: Default;
}

struct FinalMatrix;

impl DependencyResolver for FinalMatrix {
    type Source = PositionRenderObj;
    type Computed = Matrix4;

    #[inline]
    fn source(scene: &Scene, id: Id) -> &Versioned<Self::Source> {
        scene.positions.get_unwrapped(id)
    }

    fn resolve_dependencies(
        scene: &Scene,
        caches: &mut SceneCaches,
        source: &Versioned<Self::Source>,
    ) -> Version {
        let (parent_id, matrix_id) = (source.parent, source.matrix);

        // Find out the max version of all the immediate and (indirect / computed) dependencies.

        // Get the _three_ versions of the elements this one is computed on.
        // a) The self position's version.
        // b) The local matrix's version.
        // c) The computed matrix of the parent (representing all its dependencies).
        let max_deps_version = source
            .updated_at
            .max(scene.matrices.get_unwrapped(matrix_id).updated_at);

        // Combine with the optional parent.
        if let Some(parent_id) = parent_id {
            // Make sure the parent is up to date.
            resolve::<Self>(scene, caches, parent_id);
            caches.positions_matrix[parent_id]
                .max_deps_version
                .max(max_deps_version)
        } else {
            max_deps_version
        }
    }

    fn computed_mut(caches: &mut SceneCaches, id: Id) -> &mut Computed<Self::Computed> {
        caches.positions_matrix.mut_or_default(id)
    }

    fn compute(scene: &Scene, caches: &mut SceneCaches, source: &Self::Source) -> Self::Computed {
        let (parent_id, matrix_id) = (source.parent, source.matrix);
        let local_matrix = &**scene.matrices.get_unwrapped(matrix_id);
        parent_id.map_or_else(
            || *local_matrix,
            |parent_id| *caches.positions_matrix[parent_id] * local_matrix,
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
            Change::Delete(id) => self.put(id, None),
            Change::Update(id, value) => {
                // A value at this index must exist, so use `rows_mut()` here.
                self.rows_mut()[*id] = Some(value)
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
            Change::Delete(id) => self.put(id, None),
            Change::Update(id, value) => {
                self.rows_mut()[*id] = Some(Versioned::new(value, version))
            }
        }
    }
}

impl Default for Computed<Matrix4> {
    fn default() -> Self {
        Self {
            validated_at: 0,
            max_deps_version: 0,
            // OO: is there a wait to use `::ZERO` / the trait `ConstZero` from num_traits for
            // example?
            value: Matrix4::zero(),
        }
    }
}

#[derive(Debug, Default)]
struct SceneCaches {
    // The result of a positioned computation.
    positions_matrix: IdTable<Computed<Matrix4>>,
}
