use derive_more::Deref;

/// An identifier that can be used to index into rows to allow fast id associative storage and
/// retrieval of objects.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deref)]
pub struct Id(usize);

#[derive(Debug, Default)]
pub struct Generator {
    next_id: usize,
    free_list: Vec<usize>,
}

impl Generator {
    pub fn acquire(&mut self) -> Id {
        if let Some(free) = self.free_list.pop() {
            return Id(free);
        }

        let this_id = self.next_id;
        self.next_id += 1;

        Id(this_id)
    }

    pub fn release(&mut self, id: Id) {
        self.free_list.push(id.0);
    }
}
