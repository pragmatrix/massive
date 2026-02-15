use std::{fmt, hash, sync::Arc};

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
// Robustness: Should probably be based on the Arc ptr.
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
    pub(crate) fn new(id: Id, value: T, change_tracker: Arc<ChangeCollector>) -> Self {
        let uploaded = T::to_change(&value);
        change_tracker.push(Change::Create(id, uploaded));

        Self {
            inner: InnerHandle {
                id,
                change_tracker,
                value: value.into(),
            }
            .into(),
        }
    }

    pub fn id(&self) -> Id {
        self.inner.id
    }

    pub fn update_if_changed(&self, update: T)
    where
        T: PartialEq,
    {
        // Robustness: This locks twice.
        if update != *self.value() {
            self.update(update)
        }
    }

    /// Update the value of the handle.
    pub fn update(&self, update: T) {
        self.inner.update(update)
    }

    // Performance: May use replace_with?
    pub fn update_with(&self, f: impl FnOnce(&mut T)) {
        // Performance: This locks twice.
        f(&mut *self.value_mut());
        self.inner.updated();
    }

    // Performance: May use replace_with?
    pub fn update_with_if_changed(&self, f: impl FnOnce(&mut T))
    where
        T: Clone + PartialEq,
    {
        // Robustness: This locks twice if changed.
        //
        // Detail: Need to separate the lock range here clearly, otherwise the mutex stays locked
        // until self.inner.updated()
        let changed = {
            let mut v = self.value_mut();
            let before = v.clone();
            f(&mut *v);
            *v != before
        };
        if changed {
            self.inner.updated();
        }
    }

    pub fn value(&self) -> MutexGuard<'_, T> {
        self.inner.value.lock()
    }

    fn value_mut(&self) -> MutexGuard<'_, T> {
        self.inner.value.lock()
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
    // OO: Some values might be too large to be duplicated between the application and the renderer.
    value: Mutex<T>,
}

impl<T: Object> InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub fn update(&self, value: T) {
        let change = T::to_change(&value);
        self.change_tracker.push(Change::Update(self.id, change));

        *self.value.lock() = value;
    }

    pub fn updated(&self) {
        let change = T::to_change(&*self.value.lock());
        self.change_tracker.push(Change::Update(self.id, change));
    }
}

impl<T: Object> Drop for InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        self.change_tracker.push(Change::Delete(self.id));
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
