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

pub mod id_generator {
    use std::{any::TypeId, sync::LazyLock};

    use parking_lot::Mutex;

    use crate::{Id, SceneChange, type_id_generator::TypeIdGenerator};

    pub fn acquire<T: 'static>() -> Id {
        global_id_generator().lock().acquire(TypeId::of::<T>())
    }

    /// ADR: Decided to use a global id generator, so that we can support multiple scenes per renderer
    /// all sharing the same id space.
    pub fn global_id_generator() -> &'static Mutex<TypeIdGenerator> {
        static ID_GEN: LazyLock<Mutex<TypeIdGenerator>> =
            LazyLock::new(|| TypeIdGenerator::default().into());

        &ID_GEN
    }

    /// Garbage collect the ids that can be re-used after the changes are applied.
    pub fn gc(changes: &[SceneChange]) {
        // Performance: May not lock the id generator if there are no destructive changes.
        let mut id_gen = global_id_generator().lock();

        // Free up all deleted ids (this is done immediately for now, but may be later done in the
        // renderer, for example to keep ids alive until animations are finished or cached resources
        // are cleaned up)
        for (type_id, id) in changes.iter().flat_map(|sc| sc.destructive_change()) {
            // Performance: Order by TypeId first to prevent expensive HashMap lookups?
            id_gen.release(type_id, id);
        }
    }
}
