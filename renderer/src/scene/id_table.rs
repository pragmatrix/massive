//! An id associated table of objects.

use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use massive_scene::Id;

/// Values of type `V`, one row per id of the object type `K`.
///
/// The key type keeps one object type's ids out of another type's table, because each object type
/// has its own id space.
#[derive(Debug)]
pub struct IdTable<K, V> {
    rows: Vec<V>,
    key: PhantomData<fn() -> K>,
}

impl<K, V> Default for IdTable<K, V> {
    fn default() -> Self {
        Self {
            rows: Default::default(),
            key: PhantomData,
        }
    }
}

impl<K, V> IdTable<K, V> {
    pub fn insert(&mut self, id: Id<K>, value: V)
    where
        V: Default,
    {
        let index = id.to_usize();
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, V::default);
        }
        self.rows[index] = value;
    }

    /// Returns a reference to a value at `id`.
    ///
    /// May resize and create defaults.
    pub fn mut_or_default(&mut self, id: Id<K>) -> &mut V
    where
        V: Default,
    {
        let index = id.to_usize();
        if index >= self.rows.len() {
            self.rows.resize_with(index + 1, || V::default())
        }

        &mut self.rows[index]
    }

    #[allow(unused)]
    pub fn iter(&self) -> impl Iterator<Item = &V> {
        self.rows.iter()
    }
}

/// Indexing into a table is only possible with a valid id.
impl<K, V> Index<Id<K>> for IdTable<K, V> {
    type Output = V;

    fn index(&self, index: Id<K>) -> &Self::Output {
        &self.rows[index.to_usize()]
    }
}

impl<K, V> IndexMut<Id<K>> for IdTable<K, V> {
    fn index_mut(&mut self, index: Id<K>) -> &mut Self::Output {
        &mut self.rows[index.to_usize()]
    }
}
