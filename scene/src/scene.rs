use crate::any_collector::AnyCollector;
use crate::{Change, Handle, Object, SceneChange};

/// Enter an object into the change queue collected by `collector`.
///
/// Application code enters through the ambient `Enter::enter()` instead, which supplies the
/// task's installed queue (ADR 0008). Naming a collector is how tests, multi-queue tasks, and
/// code outside a task context enter explicitly; the handle retypes its changes into that
/// queue's change type via the erased sink, so entering through an instance task's collector
/// wraps every subsequent handle update and drop into the instance's change queue.
pub fn enter<T>(collector: &AnyCollector, value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<Change<T::Change>>,
{
    Handle::new(value, collector.sink().clone())
}

/// Enter a pair of dependent objects with one batch of create changes.
///
/// The second value is built from the first handle, which is how a location is entered together
/// with the transform it refers to. Both creates reach the queue under a single lock acquisition
/// and in entry order, so the first object exists before the second refers to it.
pub(crate) fn enter_pair<A, B, F>(
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
