
#[derive(Debug, Copy, Clone)]
pub struct Id(u64);

#[derive(Debug, Default)]
pub struct IdGen {
    next_id: u64,
    free_list: Vec<u64>,
}

impl IdGen {
    pub fn allocate(&mut self) -> Id {
        if let Some(free) = self.free_list.pop() {
            return Id(free);
        }

        let this_id = self.next_id;
        self.next_id += 1;

        Id(this_id)
    }

    pub fn free(&mut self, id: Id) {
        self.free_list.push(id.0);
    }
}
