use std::{mem, time::Instant};

use derive_more::Deref;
use parking_lot::Mutex;

use crate::SceneChange;

#[derive(Debug, Default)]
pub struct ChangeCollector {
    changes: Mutex<SceneChanges>,
}

impl ChangeCollector {
    pub fn push(&self, change: impl Into<SceneChange>) {
        let change = change.into();
        self.changes.lock().push(change);
    }

    pub fn push_many(&self, changes: impl Into<SceneChanges>) {
        self.changes.lock().push_many(changes.into());
    }

    pub fn take_all(&self) -> SceneChanges {
        // Performance: Preserve capacity here?
        mem::take(&mut self.changes.lock())
    }
}

#[derive(Debug, Default, Deref)]
pub struct SceneChanges {
    pub time_of_oldest_change: Option<Instant>,
    #[deref]
    pub changes: Vec<SceneChange>,
}

impl SceneChanges {
    pub fn push(&mut self, change: SceneChange) {
        if self.changes.is_empty() {
            self.time_of_oldest_change = Some(Instant::now())
        }
        self.changes.push(change);
    }

    pub fn push_many(&mut self, changes: SceneChanges) {
        match (self.time_of_oldest_change, changes.time_of_oldest_change) {
            (None, _) => {
                // Performance: Capacity
                *self = changes;
            }
            (Some(time), Some(time_new)) => {
                self.time_of_oldest_change = Some(time.min(time_new));
                self.changes.extend(changes.changes);
            }
            (Some(_), None) => {}
        }
    }

    pub fn into_inner(self) -> Option<(Instant, Vec<SceneChange>)> {
        self.time_of_oldest_change.map(|time| {
            assert!(!self.is_empty());
            (time, self.changes)
        })
    }
}
