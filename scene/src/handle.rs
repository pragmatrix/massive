use std::{fmt, hash, ops::Deref, sync::Arc};

use parking_lot::{Mutex, MutexGuard};

use crate::{Change, ChangeSink, Id, SceneChange, id_generator};

/// A handle is a mutable representation of an object entered into a scene.
///
/// Although all scenes share a common id space, a handle can only be entered into one scene.
#[derive(Debug)]
pub struct Handle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    inner: Arc<InnerHandle<T>>,
}

impl<T: Object> Clone for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// PartialEq implements reference equality based on the Id.
//
// Robustness: Should probably be based on the Arc pointer.
impl<T: Object> PartialEq for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner.id.eq(&other.inner.id)
    }
}

impl<T: Object> Eq for Handle<T> where SceneChange: From<Change<T::Change>> {}

impl<T: Object> hash::Hash for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.inner.id.hash(state);
    }
}

impl<T: Object> Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub(crate) fn new(value: T, change_collector: Arc<dyn ChangeSink>) -> Self
    where
        T: 'static,
    {
        let (handle, create) = Self::unpublished(value, &change_collector);
        change_collector.send(create.into());

        handle
    }

    /// Construct a handle without publishing its create change, returning that change.
    ///
    /// Invariant: the returned change must reach the queue before the handle is observable by
    /// other code, otherwise a later drop deletes an id the queue never saw created. The only
    /// caller is `scene::enter_pair`, which publishes the creates of both handles in one batch.
    pub(crate) fn unpublished(
        value: T,
        change_collector: &Arc<dyn ChangeSink>,
    ) -> (Self, Change<T::Change>)
    where
        T: 'static,
    {
        // Ids are keyed by type, which is what needs the 'static bound here.
        let id = id_generator::acquire::<T>();
        let create = Change::Create(id, T::to_change(&value));

        let handle = Self {
            inner: InnerHandle {
                id,
                change_collector: change_collector.clone(),
                value: value.into(),
            }
            .into(),
        };

        (handle, create)
    }

    pub fn id(&self) -> Id {
        self.inner.id
    }

    pub fn to_ref(&self) -> Ref<T> {
        Ref {
            inner: self.inner.clone(),
        }
    }

    pub fn update_if_changed(&self, update: T)
    where
        T: PartialEq,
    {
        self.inner.update_if_changed(update)
    }

    /// Update the value of the handle.
    pub fn update(&self, update: T) {
        self.inner.update(update)
    }

    pub fn update_with(&self, f: impl FnOnce(&mut T)) {
        self.inner.update_with(f);
    }

    pub fn update_if_changed_with(&self, f: impl FnOnce(&mut T))
    where
        T: Clone + PartialEq,
    {
        self.inner.update_if_changed_with(f);
    }

    pub fn value(&self) -> HandleValue<'_, T> {
        HandleValue {
            value: self.inner.value.lock(),
        }
    }
}

/// A read-only handle to an object entered into a scene.
#[derive(Debug)]
pub struct Ref<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    inner: Arc<InnerHandle<T>>,
}

impl<T: Object> Clone for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Object> PartialEq for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner.id.eq(&other.inner.id)
    }
}

impl<T: Object> Eq for Ref<T> where SceneChange: From<Change<T::Change>> {}

impl<T: Object> hash::Hash for Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.inner.id.hash(state);
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

impl<T: Object> Ref<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub fn id(&self) -> Id {
        self.inner.id
    }

    pub fn value(&self) -> HandleValue<'_, T> {
        HandleValue {
            value: self.inner.value.lock(),
        }
    }
}

#[derive(Debug)]
pub struct HandleValue<'a, T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    value: MutexGuard<'a, T>,
}

impl<T: Object> Deref for HandleValue<'_, T>
where
    SceneChange: From<Change<T::Change>>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// Internal representation of the object handle.
struct InnerHandle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
    /// This is effectively the connection to the queue it was entered into.
    change_collector: Arc<dyn ChangeSink>,
    // Optimization: Some values might be too large to be duplicated between the application and the
    // renderer.
    value: Mutex<T>,
}

impl<T: Object> fmt::Debug for InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    // The shared sink can hold the task's full pending queue, which every handle would repeat.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.value.lock();
        formatter
            .debug_struct("InnerHandle")
            .field("id", &self.id)
            .field("value", &*value)
            .finish_non_exhaustive()
    }
}

impl<T: Object> InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    // Invariant: mutate the value and enqueue its update while holding the same lock so change
    // ordering matches committed state under concurrent writers.
    pub fn update(&self, value: T) {
        self.update_locked(|current| {
            *current = value;
            true
        });
    }

    pub fn update_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        self.update_locked(|current| {
            if *current == value {
                return false;
            }

            *current = value;
            true
        });
    }

    pub fn update_with(&self, f: impl FnOnce(&mut T)) {
        self.update_locked(|current| {
            f(current);
            true
        });
    }

    pub fn update_if_changed_with(&self, f: impl FnOnce(&mut T))
    where
        T: Clone + PartialEq,
    {
        self.update_locked(|current| {
            let before = current.clone();
            f(current);
            *current != before
        });
    }

    fn update_locked(&self, mutate: impl FnOnce(&mut T) -> bool) {
        let mut current = self.value.lock();
        if !mutate(&mut *current) {
            return;
        }

        let change = T::to_change(&*current);
        self.change_collector
            .send(Change::Update(self.id, change).into());
    }
}

impl<T: Object> Drop for InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        self.change_collector.send(Change::Delete(self.id).into());
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
