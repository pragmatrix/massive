use std::{cell::RefCell, ops::Deref, rc::Rc};

use crate::{ChangeTracker, Id, Object};

/// A timeline is an exlusiviely owned object stored in the client.
///
/// A timeline is a prerequisite for animations.
///
/// Compared to a [`crate::Handle`], the timeline does not split its parts. It preserves the full
/// value at the current time in the client.
#[derive(Debug)]
pub struct Timeline<T: Object> {
    id: Id,
    value: T,
    change_tracker: Rc<RefCell<ChangeTracker>>,
}

impl<T: Object> Drop for Timeline<T> {
    fn drop(&mut self) {
        self.change_tracker.borrow_mut().delete::<T>(self.id);
    }
}

impl<T: Object> Deref for Timeline<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Object + Clone> Timeline<T> {
    pub(crate) fn new(id: Id, value: T, change_tracker: Rc<RefCell<ChangeTracker>>) -> Self {
        let (_, uploaded) = T::split(value.clone());
        change_tracker.borrow_mut().create::<T>(id, uploaded);
        Self {
            id,
            value,
            change_tracker,
        }
    }

    pub fn update(&self, value: T) {
        // OO: May find a more efficient way to extract the uploaded part, but check if the compiler
        // is able to prevent cloning the kept parts.
        let (_, uploaded) = T::split(value.clone());
        self.change_tracker
            .borrow_mut()
            .update::<T>(self.id, uploaded);
    }
}
