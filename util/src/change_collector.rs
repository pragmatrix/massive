use std::mem;

use derive_more::Deref;
use parking_lot::Mutex;

#[derive(Debug)]
pub struct ChangeCollector<C> {
    changes: Mutex<ChangeSet<C>>,
}

impl<C> Default for ChangeCollector<C> {
    fn default() -> Self {
        Self {
            changes: Mutex::new(ChangeSet::default()),
        }
    }
}

impl<C> ChangeCollector<C> {
    pub fn collect(&self, change: impl Into<C>) {
        let change = change.into();
        self.changes.lock().push(change);
    }

    pub fn collect_many(&self, changes: impl Into<ChangeSet<C>>) {
        let changes = changes.into();
        self.changes.lock().accumulate(changes);
    }

    /// Temporary API: this exists while scenes can still be created without
    /// requiring a separately owned change collector.
    pub fn take_all(&self) -> ChangeSet<C> {
        // Performance: Preserve capacity here?
        mem::take(&mut self.changes.lock())
    }
}

#[derive(Debug, Deref)]
pub struct ChangeSet<C> {
    #[deref]
    pub changes: Vec<C>,

    #[cfg(feature = "metrics")]
    pub time_of_oldest_change: Option<Instant>,
}

impl<C> Default for ChangeSet<C> {
    fn default() -> Self {
        Self {
            changes: Vec::new(),

            #[cfg(feature = "metrics")]
            time_of_oldest_change: None,
        }
    }
}

impl<C> Drop for ChangeSet<C> {
    fn drop(&mut self) {
        if !self.changes.is_empty() {
            log::error!("{} changes were not processed", self.changes.len());
        }
    }
}

impl<C> ChangeSet<C> {
    pub fn push(&mut self, change: C) {
        #[cfg(feature = "metrics")]
        {
            if self.changes.is_empty() {
                self.time_of_oldest_change = Some(Instant::now());
            }
        }

        self.changes.push(change);
    }

    pub fn accumulate(&mut self, mut changes: ChangeSet<C>) {
        if self.changes.is_empty() {
            // Performance: Capacity
            *self = changes;
        } else {
            #[cfg(feature = "metrics")]
            {
                if let Some(time_new) = changes.time_of_oldest_change {
                    self.time_of_oldest_change = Some(
                        self.time_of_oldest_change
                            .map_or(time_new, |time| time.min(time_new)),
                    );
                }
            }

            self.changes.extend(mem::take(&mut changes.changes));
        }
    }

    pub fn map<M>(mut self, map_change: impl FnMut(C) -> M) -> ChangeSet<M> {
        ChangeSet {
            // take is needed because of Drop.
            changes: mem::take(&mut self.changes)
                .into_iter()
                .map(map_change)
                .collect(),

            #[cfg(feature = "metrics")]
            time_of_oldest_change: self.time_of_oldest_change,
        }
    }

    pub fn release(mut self) -> Vec<C> {
        mem::take(&mut self.changes)
    }

    #[cfg(feature = "metrics")]
    pub fn time_of_oldest_change(&self) -> Option<Instant> {
        self.time_of_oldest_change
    }
}
