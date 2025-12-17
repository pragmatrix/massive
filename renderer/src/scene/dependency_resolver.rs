use crate::{
    Version,
    tools::{Computed, Versioned},
};

use massive_scene::Id;

/// Resolve a computed value.
///
/// Invoking this function ensures that the computed value `id` is up to date with its dependencies
/// at `head_version`.
///
/// We don't return a reference to the computed value, because the borrow checker would not be able
/// to un-borrow `computed_storage` after a return (a current limitation).
///
/// Instead we return the Version of the computed value (the last time it was recomputed, which is
/// equivalent upon return to to max_deps_version)
///
/// Robustness: Un-recurse this. There might be degenerate cases of large dependency chains.
pub fn resolve<Resolver: DependencyResolver>(
    head_version: Version,
    shared_storage: &Resolver::SourceStorage,
    computed_storage: &mut Resolver::ComputedStorage,
    id: Id,
) -> Version
where
    Computed<Resolver::Computed>: Default,
{
    // Already validated at the latest version? Done.
    //
    // This is the only situation in which the id might make the underlying storage to resize.
    let computed = Resolver::computed_mut(computed_storage, id);
    let current_max_deps = computed.versioned.updated_at;
    if computed.validated_at == head_version {
        return current_max_deps;
    }

    let source = Resolver::get_source(shared_storage, id);
    let max_deps_version =
        Resolver::resolve_dependencies(head_version, source, shared_storage, computed_storage);

    // If the max_deps_version is smaller or equal to the one of the computed value, the value is ok
    // and can be marked as validated at `head_version`.
    if max_deps_version <= current_max_deps {
        Resolver::computed_mut(computed_storage, id).validated_at = head_version;
        return current_max_deps;
    }

    // Compute a new value and store it.
    let new_value = Resolver::compute(shared_storage, computed_storage, source);
    *Resolver::computed_mut(computed_storage, id) = Computed {
        validated_at: head_version,
        versioned: Versioned::new(new_value, max_deps_version),
    };
    max_deps_version
}

pub trait DependencyResolver {
    /// Type of the source table storage.
    type SourceStorage;
    /// The symmetric _versioned_ source value type. There must be a source value for every computed
    /// value with the same id.
    type Source;

    /// Type of the computed table storage.
    type ComputedStorage;
    /// The computed value type (must implement Default for now, use Option<> otherwise)
    type Computed;

    /// Retrieve a reference to the versioned source value.
    fn get_source(scene: &Self::SourceStorage, id: Id) -> &Versioned<Self::Source>;

    /// Make sure that all dependencies are up to date and return their maximum version.
    fn resolve_dependencies(
        head_version: Version,
        source: &Versioned<Self::Source>,
        shared: &Self::SourceStorage,
        computed: &mut Self::ComputedStorage,
    ) -> Version;

    fn compute(
        shared: &Self::SourceStorage,
        computed: &Self::ComputedStorage,
        source: &Self::Source,
    ) -> Self::Computed;

    fn computed_mut(computed: &mut Self::ComputedStorage, id: Id) -> &mut Computed<Self::Computed>
    where
        Computed<Self::Computed>: Default;
}
