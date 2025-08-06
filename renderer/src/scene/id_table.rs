//! An id associated table of objects.

use std::ops::{Index, IndexMut};

use massive_scene::Id;

#[derive(Debug)]
pub struct IdTable<T> {
    rows: Vec<T>,
}

impl<T> Default for IdTable<T> {
    fn default() -> Self {
        Self {
            rows: Default::default(),
        }
    }
}

impl<T> IdTable<T> {
    pub fn insert(&mut self, id: Id, value: T)
    where
        T: Default,
    {
        let index = id.to_usize();
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, T::default);
        }
        self.rows[index] = value;
    }

    /// Returns a reference to a value at `id`.
    ///
    /// May resize and create defaults.
    pub fn mut_or_default(&mut self, id: Id) -> &mut T
    where
        T: Default,
    {
        let index = id.to_usize();
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, || T::default())
        }

        &mut self.rows[index]
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.rows.iter()
    }
}

/// Indexing into a table is only possible with a valid id.
impl<T> Index<Id> for IdTable<T> {
    type Output = T;

    fn index(&self, index: Id) -> &Self::Output {
        &self.rows[index.to_usize()]
    }
}

impl<T> IndexMut<Id> for IdTable<T> {
    fn index_mut(&mut self, index: Id) -> &mut Self::Output {
        &mut self.rows[index.to_usize()]
    }
}
