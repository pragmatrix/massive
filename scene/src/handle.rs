use std::{fmt, hash, marker::PhantomData};

use crate::{Change, Id, SceneChange, current_queue, id_generator};

/// The single owner and writer of an object submitted to a queue.
///
/// Its changes go to the change queue of the task it is used in, so it must be used inside a task
/// (ADR 0040). A handle is not `Clone`, because a clone would hold its own copy of the value, and
/// two writers would overwrite each other's snapshots. Code that only keeps the object alive holds
/// a [`Ref`].
pub struct Handle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
    // Optimization: Some values might be too large to be duplicated between the application and the
    // renderer.
    value: T,
}

impl<T: Object> fmt::Debug for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("id", &self.id)
            .field("value", &self.value)
            .finish()
    }
}

impl<T: Object> Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub(crate) fn new(value: T) -> Self
    where
        T: 'static,
    {
        let (handle, create) = Self::unpublished(value);
        current_queue::send(create.into());

        handle
    }

    /// Construct a handle without publishing its create change, returning that change.
    ///
    /// Invariant: the returned change must reach the queue before the handle is observable by
    /// other code, otherwise a later release refers to an id the lifetime stage never saw created.
    #[must_use = "the returned create change must be published, or the handle's id is unknown to the queue"]
    pub(crate) fn unpublished(value: T) -> (Self, Change<T::Change>)
    where
        T: 'static,
    {
        // Ids are keyed by type, which is what needs the 'static bound here.
        let id = id_generator::acquire::<T>();
        let create = Change::Create(id, T::to_change(&value));
        (Self { id, value }, create)
    }

    pub fn id(&self) -> Id {
        self.id
    }

    /// A counted reference that keeps the object alive, also after this handle is dropped.
    pub fn to_ref(&self) -> Ref<T> {
        Ref::retain(self.id)
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    /// Update the value of the handle.
    pub fn update(&mut self, value: T) {
        self.value = value;
        self.send_update();
    }

    pub fn update_if_changed(&mut self, value: T)
    where
        T: PartialEq,
    {
        if self.value != value {
            self.update(value);
        }
    }

    pub fn update_with(&mut self, f: impl FnOnce(&mut T)) {
        f(&mut self.value);
        self.send_update();
    }

    /// Set one field of the value, and send the update only when the field changes.
    ///
    /// `field` selects the field, so its current value and `value` can be compared directly.
    pub(crate) fn set_if_changed<F: PartialEq>(
        &mut self,
        field: impl FnOnce(&mut T) -> &mut F,
        value: F,
    ) {
        let slot = field(&mut self.value);
        if *slot == value {
            return;
        }
        *slot = value;
        self.send_update();
    }

    fn send_update(&self) {
        let change = T::to_change(&self.value);
        current_queue::send(Change::Update(self.id, change).into());
    }
}

impl<T: Object> Drop for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        current_queue::send(Change::Release(self.id).into());
    }
}

/// A counted reference that keeps an object alive.
///
/// Each reference counts once in the lifetime stage, which deletes the object only after its handle
/// and every reference are dropped (ADR 0040). A reference carries only the object's id, so it
/// cannot read the value.
pub struct Ref<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
    object: PhantomData<fn() -> T>,
}

impl<T: Object> Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub fn id(&self) -> Id {
        self.id
    }

    fn retain(id: Id) -> Self {
        current_queue::send(Change::<T::Change>::Retain(id).into());
        Self {
            id,
            object: PhantomData,
        }
    }
}

impl<T: Object> fmt::Debug for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Ref").field("id", &self.id).finish()
    }
}

impl<T: Object> Clone for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn clone(&self) -> Self {
        Self::retain(self.id)
    }
}

impl<T: Object> Drop for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        current_queue::send(Change::<T::Change>::Release(self.id).into());
    }
}

impl<T: Object> PartialEq for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl<T: Object> Eq for Ref<T> where SceneChange: From<Change<T::Change>> {}

impl<T: Object> hash::Hash for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: Object> From<Handle<T>> for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn from(value: Handle<T>) -> Self {
        value.to_ref()
    }
}

impl<T: Object> From<&Handle<T>> for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn from(value: &Handle<T>) -> Self {
        value.to_ref()
    }
}

impl<T: Object> From<&Ref<T>> for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn from(value: &Ref<T>) -> Self {
        value.clone()
    }
}

pub trait Object: Sized + fmt::Debug
where
    SceneChange: From<Change<Self::Change>>,
{
    /// The type of the change the renderer needs to receive.
    type Change;

    /// Convert the current value to something that can be uploaded.
    fn to_change(&self) -> Self::Change;
}
