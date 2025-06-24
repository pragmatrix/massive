use std::{cell::RefCell, fmt, rc::Rc};

use crate::{Change, ChangeTracker, Id, SceneChange};

pub trait Object: Sized
where
    SceneChange: From<Change<Self::Change>>,
{
    /// The stuff from Self that needs to be stored locally to keep the referential integrity. These
    /// are the handles this instance refers to and which need to be kept alive.
    type Keep: fmt::Debug;
    /// The type of the change the renderer needs to receive.
    type Change;

    /// Separate the part we need to keep from the part to be uploaded.
    fn split(self) -> (Self::Keep, Self::Change);
}

#[derive(Debug)]
pub struct Handle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    inner: Rc<InnerHandle<T>>,
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

impl<T: Object> Handle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub(crate) fn new(id: Id, value: T, change_tracker: Rc<RefCell<ChangeTracker>>) -> Self {
        let (pinned, uploaded) = T::split(value);
        change_tracker
            .borrow_mut()
            .push(Change::Create(id, uploaded));

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
struct InnerHandle<T: Object>
where
    SceneChange: From<Change<T::Change>>,
{
    id: Id,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    // As long as the inner handle is referred to by a bare Rc<>, we need a RefCell here. TODO:
    // Since T::Pinned is also Rc, we could put this directly in Handle, as we could the Id, it
    // would make the handle quite fat though, but is this a problem assuming that the number of
    // handles are usually only one on average ... but not inside of shapes?! Also: how frequent is
    // update being called?
    pinned: RefCell<T::Keep>,
}

impl<T: Object> InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    pub fn update(&self, value: T) {
        let (pinned, uploaded) = T::split(value);
        self.change_tracker
            .borrow_mut()
            .push(Change::Update(self.id, uploaded));

        *self.pinned.borrow_mut() = pinned;
    }
}

impl<T: Object> Drop for InnerHandle<T>
where
    SceneChange: From<Change<T::Change>>,
{
    fn drop(&mut self) {
        self.change_tracker
            .borrow_mut()
            .push(Change::Delete(self.id));
    }
}
