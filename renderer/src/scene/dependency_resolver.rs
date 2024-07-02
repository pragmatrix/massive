use super::versioning::{Computed, Version, Versioned};
use massive_scene::Id;

/// Resolve a computed value.
///
/// Invoking this function ensures that the computed value `id` is up to date with its dependencies
/// at `head_version`.
///
/// We don't return a reference to the computed value, because the borrow checker would not be
/// able to unborrow `computed_storage` after a return (a current limitation).
///
/// TODO: Unrecurse this. There might be degenerate cases of large dependency chains.
pub fn resolve<Resolver: DependencyResolver>(
    head_version: Version,
    shared_storage: &Resolver::SharedStorage,
    computed_storage: &mut Resolver::ComputedStorage,
    id: Id,
) where
    Computed<Resolver::Computed>: Default,
{
    // Already validated at the latest version? Done.
    //
    // `get_or_default` must be used here. This is the only situation in which the cache may
    // need to be resized.
    if Resolver::computed_mut(computed_storage, id).validated_at == head_version {
        return;
    }

    // Save the current max dependencies version for later.
    //
    // In theory this could be overwritten if there are cycles in the dependency graph, but in
    // practice they are not (and everything may blow up anyway).
    let computed_max_deps = Resolver::computed_mut(computed_storage, id).max_deps_version;

    let source = Resolver::source(shared_storage, id);
    let max_deps_version =
        Resolver::resolve_dependencies(head_version, source, shared_storage, computed_storage);

    // If the max_deps_version is smaller or equal to the one of the computed value, the value is ok
    // and can be marked as validated at `head_version`.
    if max_deps_version <= computed_max_deps {
        Resolver::computed_mut(computed_storage, id).validated_at = head_version;
        return;
    }

    // Compute a new value and store it.
    let new_value = Resolver::compute(shared_storage, computed_storage, source);
    *Resolver::computed_mut(computed_storage, id) = Computed {
        validated_at: head_version,
        max_deps_version,
        value: new_value,
    };
}

pub trait DependencyResolver {
    /// Type of the shared table storage.
    type SharedStorage;
    /// Type of the computed table storage.
    type ComputedStorage;

    /// The symmetric _versioned_ source value type. There must be a source value for every computed
    /// value with the same id.
    type Source;
    /// The computed value type (must implement Default for now, use Option<> otherwise)
    type Computed;

    /// Retrieve a reference to the versioned source value.
    fn source(scene: &Self::SharedStorage, id: Id) -> &Versioned<Self::Source>;

    /// Make sure that all dependencies are up to date and return their maximum version.
    fn resolve_dependencies(
        head_version: Version,
        source: &Versioned<Self::Source>,
        shared: &Self::SharedStorage,
        computed: &mut Self::ComputedStorage,
    ) -> Version;

    fn compute(
        shared: &Self::SharedStorage,
        computed: &Self::ComputedStorage,
        source: &Self::Source,
    ) -> Self::Computed;

    fn computed_mut(computed: &mut Self::ComputedStorage, id: Id) -> &mut Computed<Self::Computed>
    where
        Computed<Self::Computed>: Default;
}
