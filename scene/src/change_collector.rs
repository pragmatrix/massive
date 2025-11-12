use std::mem;

use parking_lot::Mutex;

use crate::SceneChange;

#[derive(Debug, Default)]
pub struct ChangeCollector(Mutex<Vec<SceneChange>>);

impl ChangeCollector {
    pub fn push(&self, change: impl Into<SceneChange>) {
        let change = change.into();
        self.0.lock().push(change);
    }

    pub fn push_many(&self, changes: Vec<SceneChange>) {
        self.0.lock().extend(changes);
    }

    pub fn take_all(&self) -> Vec<SceneChange> {
        mem::take(&mut self.0.lock())
    }
}
