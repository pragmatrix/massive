use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{Change, ChangeCollector, Id, SceneChange};

pub trait Object: Sized + fmt::Debug
where
    SceneChange: From<Change<Self::Change>>,
{
    /// The type of the change the renderer needs to receive.
    type Change;

    /// Convert the current value to something that can be uploaded.
    fn to_change(&self) -> Self::Change;
}

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

impl<T: Object> PartialEq for Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner.id.eq(&other.inner.id)
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
        if update != *self.value() {
            self.update(update)
        }
    }

    /// Update the value of the handle.
    pub fn update(&self, update: T) {
        self.inner.update(update)
    }

    pub fn update_with(&self, f: impl FnOnce(&mut T)) {
        f(&mut *self.value_mut());
        self.inner.updated();
    }

    pub fn value(&self) -> MutexGuard<'_, T> {
        self.inner.value.lock().unwrap()
    }

    fn value_mut(&self) -> MutexGuard<'_, T> {
        self.inner.value.lock().unwrap()
    }
}

/// Internal representation of the object handle.
#[derive(Debug)]
struct InnerHandle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
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

        *self.value.lock().unwrap() = value;
    }

    pub fn updated(&self) {
        let change = T::to_change(&*self.value.lock().unwrap());
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
