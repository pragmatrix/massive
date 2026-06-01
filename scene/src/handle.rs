use std::{fmt, hash, ops::Deref, sync::Arc};

use parking_lot::{Mutex, MutexGuard};

use crate::{Change, ChangeCollector, Id, Scene, SceneChange};

/// A handle is a mutable representation of an object staged on a scene.
///
/// Although all scenes share a common id space, a handle can only be staged on one scene.
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
    pub(crate) fn new(id: Id, value: T, change_collector: Arc<ChangeCollector>) -> Self {
        let uploaded = T::to_change(&value);
        change_collector.collect(Change::Create(id, uploaded));

        Self {
            inner: InnerHandle {
                id,
                change_tracker: change_collector,
                value: value.into(),
            }
            .into(),
        }
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

/// A read-only handle to an object staged on a scene.
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
#[derive(Debug)]
struct InnerHandle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
    /// This is effectively the connection to the scene it was staged in.
    change_tracker: Arc<ChangeCollector>,
    // Optimization: Some values might be too large to be duplicated between the application and the
    // renderer.
    value: Mutex<T>,
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
        self.change_tracker.collect(Change::Update(self.id, change));
    }
}

impl<T: Object> Drop for InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        self.change_tracker.collect(Change::Delete(self.id));
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

    fn enter(self, scene: &Scene) -> Handle<Self>
    where
        Self: 'static,
    {
        scene.stage(self)
    }
}
