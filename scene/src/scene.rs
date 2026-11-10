use crate::{Change, Handle, Object, SceneChange, current_queue};

/// Submit an object to the current task's change queue, making it active there (ADR 0040).
pub fn submit<T>(value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<Change<T::Change>>,
{
    Handle::new(value)
}

/// Submit a pair of dependent objects with one batch of create changes, the second built from the
/// first handle — which is how a location is submitted together with its transform. Both creates
/// reach the queue in submission order, so the first object exists before the second refers to it.
pub(crate) fn submit_pair<A, B, F>(first: A, second: F) -> (Handle<A>, Handle<B>)
where
    A: Object + 'static,
    B: Object + 'static,
    F: FnOnce(&Handle<A>) -> B,
    SceneChange: From<Change<A::Change>>,
    SceneChange: From<Change<B::Change>>,
{
    let (first_handle, first_create) = Handle::unpublished(first);

    // The second value is derived from the first handle: this is the dependency the ordered batch
    // preserves.
    let second_value = second(&first_handle);
    let (second_handle, second_create) = Handle::unpublished(second_value);

    current_queue::send_all([first_create.into(), second_create.into()]);

    (first_handle, second_handle)
}
