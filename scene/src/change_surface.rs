//! Change Surfaces
//!
//! The basic idea of a change surface is an imagined _geometrical_ abstraction of a set of changes
//! that is decoupled (as far as possible) from the original changes and can be used as a cheap
//! transport mechanism to inform other _derived_ computations what have changed so that they can
//! rebuild their caches based on the original changes or a concrete version of the changes (the
//! model, or perhaps an aggregate).
//!
//! A change surface contains all information about what has _structurally_ be changed. The nature
//! of the changes itself is not encoded (i.e. insert, delete, or update).
//!
//! A change surface is coarse grained, it may contain a superset of the changes. This means that
//! for expensive updates, it may make sense to verify if there is actually a change before updating
//! derived data.
//!
//! Compared to the data types, a change surface supports only a very limited number of functions to
//! clarify to clients how to use it.

use std::collections::HashSet;

use crate::Id;

/// A change surface for a set of ids.
// This is currently implemented as a HashSet.
//
// Performance: A range set / interval set might be an optimization opportunity here.
#[derive(Debug, Default)]
pub struct ChangedIds {
    changed: HashSet<Id>,
}

impl ChangedIds {
    /// Add a single id to the change surface.
    pub fn add(&mut self, id: Id) {
        self.changed.insert(id);
    }

    /// Take all the changes out of the change surface.
    pub fn take_all(&mut self) -> impl Iterator<Item = Id> {
        self.changed.drain()
    }
}
