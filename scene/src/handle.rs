use std::{cell::RefCell, fmt, rc::Rc};

use crate::{Change, ChangeTracker, Id, SceneChange};

pub trait Object: Sized {
    /// The stuff from Self that needs to be stored locally to keep the referential integrity. These
    /// are the handles this instance refers to and which need to be kept alive.
    type Keep: fmt::Debug;
    /// The type Uploaded type to the renderer.
    type Change;

    // TODO: It's possible to use a const here.
    // const INTO_SCENE_CHANGE: fn(Change<Self>) -> SceneChange;
    fn promote_change(change: Change<Self::Change>) -> SceneChange;

    /// Separate the part we need to keep from the part to be uploaded.
    fn split(self) -> (Self::Keep, Self::Change);
}

#[derive(Debug, Clone)]
pub struct Handle<T: Object> {
    inner: Rc<InnerHandle<T>>,
}

impl<T: Object> Handle<T> {
    pub(crate) fn new(id: Id, value: T, change_tracker: Rc<RefCell<ChangeTracker>>) -> Self {
        let (pinned, uploaded) = T::split(value);
        change_tracker.borrow_mut().create::<T>(id, uploaded);

        Self {
            inner: InnerHandle {
                id,
                change_tracker,
                pinned: pinned.into(),
            }
            .into(),
        }
    }

    pub fn id(&self) -> Id {
        self.inner.id
    }

    /// Update the value of the handle.
    pub fn update(&self, update: T) {
        self.inner.update(update)
    }
}

/// Internal representation of the object handle.
#[derive(Debug)]
struct InnerHandle<T: Object> {
    id: Id,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    // As long the inner handle is referred to by a bare Rc<>, we need a RefCell here. TODO: Since
    // T::Pinned is also Rc, we could put this directly in Handle, as we could the Id, it would make
    // the handle quite fat though, but is this a problem assuming that the number of handles are
    // usually only one on average ... but not inside of shapes?!
    // Also: how frequent is update being called?
    pinned: RefCell<T::Keep>,
}

impl<T: Object> InnerHandle<T> {
    pub fn update(&self, value: T) {
        let (pinned, uploaded) = T::split(value);
        self.change_tracker
            .borrow_mut()
            .update::<T>(self.id, uploaded);

        *self.pinned.borrow_mut() = pinned;
    }
}

impl<T: Object> Drop for InnerHandle<T> {
    fn drop(&mut self) {
        self.change_tracker.borrow_mut().delete::<T>(self.id);
    }
}
