use std::{mem, ops::DerefMut, sync::Mutex};

use crate::SceneChange;

#[derive(Debug, Default)]
pub struct ChangeCollector(Mutex<Vec<SceneChange>>);

impl ChangeCollector {
    pub fn push(&self, change: impl Into<SceneChange>) {
        let change = change.into();
        self.0.lock().unwrap().push(change);
    }

    pub fn push_many(&self, changes: Vec<SceneChange>) {
        self.0.lock().unwrap().extend(changes);
    }

    pub fn take_all(&self) -> Vec<SceneChange> {
        mem::take(self.0.lock().unwrap().deref_mut())
    }
}
