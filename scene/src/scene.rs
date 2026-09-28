use crate::any_collector::AnyCollector;
use crate::{Change, Handle, Object, SceneChange};

/// Submit an object to the queue whose changes `collector` collects, making it active there.
///
/// Naming a collector is how tests, multi-queue tasks, and code outside a task context submit
/// explicitly.
pub fn submit<T>(collector: &AnyCollector, value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<Change<T::Change>>,
{
    Handle::new(value, collector.sink().clone())
}

/// Submit a pair of dependent objects with one batch of create changes, the second built from the
/// first handle — which is how a location is submitted together with its transform. Both creates
/// reach the queue in submission order, so the first object exists before the second refers to it.
pub(crate) fn submit_pair<A, B, F>(
    collector: &AnyCollector,
    first: A,
    second: F,
) -> (Handle<A>, Handle<B>)
where
    A: Object + 'static,
    B: Object + 'static,
    F: FnOnce(&Handle<A>) -> B,
    SceneChange: From<Change<A::Change>>,
    SceneChange: From<Change<B::Change>>,
{
    let sink = collector.sink().clone();

    let (first_handle, first_create) = Handle::unpublished(first, &sink);

    // The second value is derived from the first handle: this is the dependency the ordered batch
    // preserves.
    let second_value = second(&first_handle);
    let (second_handle, second_create) = Handle::unpublished(second_value, &sink);

    sink.send_all(&mut [first_create.into(), second_create.into()].into_iter());

    (first_handle, second_handle)
}
