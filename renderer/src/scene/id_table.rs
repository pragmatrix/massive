//! An id associated table of objects.

use std::{mem, ops::Index};

use massive_scene::{Change, Id};

#[derive(Debug)]
pub struct IdTable<T> {
    rows: Vec<Option<T>>,
}

impl<T> Default for IdTable<T> {
    fn default() -> Self {
        Self {
            rows: Default::default(),
        }
    }
}

impl<T> IdTable<T> {
    pub fn apply(&mut self, change: Change<T>) {
        match change {
            Change::Create(id, value) => self.put(id, value),
            Change::Delete(id) => self.remove(id),
            Change::Update(id, value) => self.rows[*id] = Some(value),
        }
    }

    pub fn put(&mut self, id: Id, value: T) {
        let index = *id;
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, || None);
        }
        self.rows[index] = Some(value);
    }

    pub fn remove(&mut self, id: Id) {
        self.rows[*id] = None;
    }

    #[allow(unused)]
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

    pub fn reset(&mut self) {
        self.rows.clear();
    }
}

/// Indexing into a table is only possible with a valid id.
impl<T> Index<Id> for IdTable<T> {
    type Output = T;

    fn index(&self, index: Id) -> &Self::Output {
        self.rows[*index].as_ref().unwrap()
    }
}
