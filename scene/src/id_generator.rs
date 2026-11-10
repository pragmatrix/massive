//! The id spaces of the scene objects, one per object kind.
//!
//! Ids are generated in the clients, because objects are created there. The renderer prefers dense
//! ids, so it can keep its data in plain vectors, and a released id is reused first (ADR 0040).

use parking_lot::Mutex;

use crate::id::Generator;
use crate::lifetime_stage::Target;
use crate::{Id, Object};

/// The generators of the three object kinds, indexed by [`Kind`](crate::Kind).
///
/// One lock for all kinds, because every task that creates objects and the lifetime stage that
/// releases them share these id spaces.
static GENERATORS: Mutex<[Generator; 3]> =
    Mutex::new([Generator::new(), Generator::new(), Generator::new()]);

pub fn acquire<T: Object>() -> Id<T> {
    Id::from_index(GENERATORS.lock()[T::KIND.index()].acquire())
}

/// Return the ids of ended objects, so later objects can reuse them. The lifetime stage calls
/// this once per batch (ADR 0040).
pub fn release_all(ended: &[Target]) {
    if ended.is_empty() {
        return;
    }

    let mut generators = GENERATORS.lock();
    for target in ended {
        generators[target.kind.index()].release(target.index);
    }
}
