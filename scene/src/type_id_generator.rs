use std::{any::TypeId, collections::HashMap};

use crate::{Id, id::Generator};

/// A (sharable) generator for ids per type.
#[derive(Debug, Default)]
pub struct TypeIdGenerator {
    ids: HashMap<TypeId, Generator>,
}

impl TypeIdGenerator {
    pub fn acquire(&mut self, tid: TypeId) -> Id {
        self.ids.entry(tid).or_default().acquire()
    }

    pub fn release(&mut self, tid: TypeId, id: Id) {
        self.ids
            .get_mut(&tid)
            .expect("Releasing an id failed, generator missing for type")
            .release(id);
    }
}

/// The global ID Generator.
///
/// ADR: Decided to use a global id generator, so that we can support multiple scenes per renderer
/// all sharing the same id space.
pub mod id_generator {
    use std::any::TypeId;
    use std::sync::LazyLock;

    use parking_lot::Mutex;

    use crate::Id;
    use crate::type_id_generator::TypeIdGenerator;

    pub fn acquire<T: 'static>() -> Id {
        global_id_generator().lock().acquire(TypeId::of::<T>())
    }

    pub fn global_id_generator() -> &'static Mutex<TypeIdGenerator> {
        static ID_GEN: LazyLock<Mutex<TypeIdGenerator>> =
            LazyLock::new(|| TypeIdGenerator::default().into());

        &ID_GEN
    }

    /// Return the ids of ended objects, keyed by their object type, so later objects can reuse
    /// them. The lifetime stage calls this once per batch (ADR 0040).
    pub fn release_all(ids: &[(TypeId, Id)]) {
        if ids.is_empty() {
            return;
        }

        let mut id_gen = global_id_generator().lock();
        for &(type_id, id) in ids {
            // Performance: Order by `TypeId` first to prevent expensive hashmap lookups?
            id_gen.release(type_id, id);
        }
    }
}
