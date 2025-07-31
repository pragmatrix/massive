use std::{any::TypeId, collections::HashMap};

use crate::{id::Generator, Id};

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
