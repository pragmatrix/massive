use crate::{Change, Handle, Object, SceneChange};

/// Submit an object to the current task's change queue, making it active there (ADR 0040).
pub fn submit<T>(value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<Change<T::Change>>,
{
    Handle::new(value)
}

/// Submit a pair of dependent objects, the second built from the first handle — which is how a
/// location is submitted together with its transform.
///
/// The first object's create reaches the queue before the second value is built, because building
/// it references the first object, and that reference's retain must follow the create (test
/// `a_submitted_location_creates_its_transform_before_referencing_it`).
pub(crate) fn submit_pair<A, B, F>(first: A, second: F) -> (Handle<A>, Handle<B>)
where
    A: Object + 'static,
    B: Object + 'static,
    F: FnOnce(&Handle<A>) -> B,
    SceneChange: From<Change<A::Change>>,
    SceneChange: From<Change<B::Change>>,
{
    let first_handle = Handle::new(first);
    let second_handle = Handle::new(second(&first_handle));
    (first_handle, second_handle)
}
