use std::mem;
use std::time::Instant;

use derive_more::Deref;
use parking_lot::Mutex;

use crate::SceneChange;
use crate::id_generator;

#[derive(Debug, Default)]
pub struct ChangeCollector {
    changes: Mutex<SceneChanges>,
}

impl ChangeCollector {
    pub fn collect(&self, change: impl Into<SceneChange>) {
        let change = change.into();
        self.changes.lock().push(change);
    }

    pub fn collect_many(&self, changes: impl Into<SceneChanges>) {
        self.changes.lock().accumulate(changes.into());
    }

    pub fn take_all(&self) -> SceneChanges {
        // Performance: Preserve capacity here?
        mem::take(&mut self.changes.lock())
    }
}

#[derive(Debug, Default, Deref)]
pub struct SceneChanges {
    #[deref]
    pub changes: Vec<SceneChange>,
    pub time_of_oldest_change: Option<Instant>,
}

impl Drop for SceneChanges {
    fn drop(&mut self) {
        if !self.changes.is_empty() {
            log::error!("{} scene changes were not processed", self.changes.len());
        }
    }
}

impl SceneChanges {
    pub fn push(&mut self, change: SceneChange) {
        if self.changes.is_empty() {
            self.time_of_oldest_change = Some(Instant::now())
        }
        self.changes.push(change);
    }

    pub fn accumulate(&mut self, mut changes: SceneChanges) {
        match (self.time_of_oldest_change, changes.time_of_oldest_change) {
            (None, _) => {
                // Performance: Capacity
                *self = changes;
            }
            (Some(time), Some(time_new)) => {
                self.time_of_oldest_change = Some(time.min(time_new));
                self.changes.extend(mem::take(&mut changes.changes));
            }
            (Some(_), None) => {}
        }
    }

    /// This converts SceneChanges into their Vec representation and frees all ids that are not used
    /// anymore.
    pub fn release(mut self) -> Option<(Instant, Vec<SceneChange>)> {
        self.time_of_oldest_change.map(|time| {
            assert!(!self.is_empty());
            id_generator::gc(&self.changes);
            (time, mem::take(&mut self.changes))
        })
    }
}
