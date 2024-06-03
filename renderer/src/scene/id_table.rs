//! An id associated table of objects.

use massive_scene::Id;
use std::mem;

#[derive(Debug, Default)]
pub struct IdTable<T> {
    rows: Vec<Option<T>>,
}

impl<T> IdTable<T> {
    pub fn put(&mut self, id: Id, value: T) {
        let index = *id;
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, || None);
        }
        self.rows[index] = Some(value);
    }

    pub fn remove(&mut self, id: Id) {
        _ = self.take(id);
    }

    #[must_use]
    pub fn take(&mut self, id: Id) -> Option<T> {
        let index = *id;
        if index < self.rows.len() {
            mem::take(&mut self.rows[index])
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.rows.iter().filter_map(|v| v.as_ref())
    }
}
