use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::{Change, ChangeTracker, Id, SceneChange};

pub trait Object: Sized {
    // TODO: It's possible to use a const here.
    // const INTO_SCENE_CHANGE: fn(Change<Self>) -> SceneChange;
    fn promote_change(change: Change<Self>) -> SceneChange;
}

#[derive(Debug, Clone)]
pub struct Handle<T: Object> {
    inner: Rc<InnerHandle<T>>,
}

impl<T: Object> Handle<T> {
    pub(crate) fn new(id: Id, value: T, change_tracker: Rc<RefCell<ChangeTracker>>) -> Self {
        change_tracker.borrow_mut().push(Change::Create(id, value));

        Self {
            inner: InnerHandle {
                id,
                change_tracker,
                pd: PhantomData,
            }
            .into(),
        }
    }

    pub fn update(&self, update: T) {
        self.inner.update(update)
    }
}

/// Internal representation of the object handle.
#[derive(Debug)]
struct InnerHandle<T: Object> {
    id: Id,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    pd: PhantomData<T>,
}

impl<T: Object> InnerHandle<T> {
    pub fn update(&self, update: T) {
        self.change_tracker
            .borrow_mut()
            .push(Change::Update(self.id, update))
    }
}

impl<T: Object> Drop for InnerHandle<T> {
    fn drop(&mut self) {
        self.change_tracker
            .borrow_mut()
            .push::<T>(Change::Delete(self.id))
    }
}
