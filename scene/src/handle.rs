use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::{Change, ChangeTracker, Id, SceneChange};

pub trait Object: Sized {
    fn promote_change(change: Change<Self>) -> SceneChange;
}


pub struct Handle<T: Object> {
    obj: Rc<InternalHandle<T>>,
}

impl<T: Object> Handle<T> {
    pub(crate) fn new(id: Id, value: T, change_tracker: Rc<RefCell<ChangeTracker>>) -> Self {
        change_tracker.borrow_mut().push(Change::Create(id, value));

        Self {
            obj: InternalHandle {
                id,
                change_tracker,
                pd: PhantomData,
            }
            .into(),
        }
    }

    pub fn update(&self, update: T) {
        self.obj.update(update)
    }
}

/// Internal representation of the object handle.
struct InternalHandle<T: Object> {
    id: Id,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    pd: PhantomData<T>,
}

impl<T: Object> InternalHandle<T> {
    pub fn update(&self, update: T) {
        self.change_tracker
            .borrow_mut()
            .push(Change::Update(self.id, update))
    }
}

impl<T: Object> Drop for InternalHandle<T> {
    fn drop(&mut self) {
        self.change_tracker
            .borrow_mut()
            .push::<T>(Change::Delete(self.id))
    }
}
