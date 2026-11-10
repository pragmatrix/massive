use std::{fmt, hash, marker::PhantomData};

use crate::{Handle, Object, Ref};

/// The identifier of a scene object of type `T`, which also indexes the renderer's tables of that
/// type.
///
/// Every object type has its own id space, which the type parameter keeps apart. Within it, ids
/// start at 0 and the id generator reuses the ids of ended objects, so they stay dense. That makes
/// them optimal as storage indices and allows them to be small (`u32`).
///
/// An object's value refers to other objects by their ids. An id does not keep its object alive by
/// itself. Once the referring object is submitted, the lifetime stage counts the reference for as
/// long as that object lives (ADR 0040). Until then, the handle or reference the id was taken from
/// has to keep the object alive.
pub struct Id<T> {
    index: u32,
    object: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub fn to_usize(self) -> usize {
        self.index as _
    }

    pub(crate) fn from_index(index: u32) -> Self {
        Self {
            index,
            object: PhantomData,
        }
    }

    pub(crate) fn index(self) -> u32 {
        self.index
    }
}

impl<T: Object> From<&Handle<T>> for Id<T> {
    fn from(handle: &Handle<T>) -> Self {
        handle.id()
    }
}

impl<T: Object> From<&Ref<T>> for Id<T> {
    fn from(reference: &Ref<T>) -> Self {
        reference.id()
    }
}

// The traits are implemented by hand, because derives would require them of `T` as well.

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for Id<T> {}

impl<T> hash::Hash for Id<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Id").field(&self.index).finish()
    }
}

/// Hands out the dense indices of one object type's ids, reusing released ones first.
#[derive(Debug)]
pub struct Generator {
    next_index: u32,
    free_list: Vec<u32>,
}

impl Generator {
    pub const fn new() -> Self {
        Self {
            next_index: 0,
            free_list: Vec::new(),
        }
    }

    pub fn acquire(&mut self) -> u32 {
        if let Some(free) = self.free_list.pop() {
            return free;
        }

        let index = self.next_index;
        self.next_index += 1;
        index
    }

    pub fn release(&mut self, index: u32) {
        self.free_list.push(index);
    }
}
