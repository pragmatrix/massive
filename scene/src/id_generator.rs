//! The id spaces of the scene objects, one per object kind.
//!
//! Ids are generated in the clients, because objects are created there. The renderer prefers dense
//! ids, so it can keep its data in plain vectors, and a released id is reused first (ADR 0040).

use parking_lot::Mutex;

use crate::object_kinds::{OBJECT_KINDS, TaggedId};
use crate::{Id, Object};

/// One generator per object kind, indexed by [`Object::INDEX`].
///
/// One lock for all kinds, because every task that creates objects and the lifetime stage that
/// releases them share these id spaces.
static GENERATORS: Mutex<[Generator; OBJECT_KINDS]> =
    Mutex::new([const { Generator::new() }; OBJECT_KINDS]);

pub fn acquire<T: Object>() -> Id<T> {
    Id::from_index(GENERATORS.lock()[T::INDEX].acquire())
}

/// Return the ids of ended objects, so later objects can reuse them.
///
/// The lifetime stage calls this once per scene change batch, after it has resolved the batch. A
/// scene change batch is the set of scene changes a window renderer takes at once, which may hold
/// several submissions of the task it renders (ADR 0040).
pub(crate) fn release_all(ended: &[TaggedId]) {
    if ended.is_empty() {
        return;
    }

    let mut generators = GENERATORS.lock();
    for id in ended {
        generators[id.kind].release(id.index);
    }
}

/// Hands out the dense indices of one object type's ids, reusing released ones first.
#[derive(Debug)]
struct Generator {
    next_index: u32,
    free_list: Vec<u32>,
}

impl Generator {
    const fn new() -> Self {
        Self {
            next_index: 0,
            free_list: Vec::new(),
        }
    }

    fn acquire(&mut self) -> u32 {
        if let Some(free) = self.free_list.pop() {
            return free;
        }

        let index = self.next_index;
        self.next_index += 1;
        index
    }

    fn release(&mut self, index: u32) {
        self.free_list.push(index);
    }
}
