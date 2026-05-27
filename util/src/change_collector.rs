use std::mem;
#[cfg(feature = "metrics")]
use std::time::Instant;

use derive_more::Deref;
use parking_lot::Mutex;

#[derive(Debug)]
pub struct ChangeCollector<C> {
    changes: Mutex<Changes<C>>,
}

impl<C> Default for ChangeCollector<C> {
    fn default() -> Self {
        Self {
            changes: Mutex::new(Changes::default()),
        }
    }
}

impl<C> ChangeCollector<C> {
    pub fn collect(&self, change: C) {
        self.changes.lock().push(change);
    }

    pub fn collect_many(&self, changes: impl Into<Changes<C>>) {
        self.changes.lock().accumulate(changes.into());
    }

    pub fn take_all(&self) -> Changes<C> {
        // Performance: Preserve capacity here?
        mem::take(&mut self.changes.lock())
    }
}

#[derive(Debug, Deref)]
pub struct Changes<C> {
    #[deref]
    pub changes: Vec<C>,

    #[cfg(feature = "metrics")]
    pub time_of_oldest_change: Option<Instant>,
}

impl<C> Default for Changes<C> {
    fn default() -> Self {
        Self {
            changes: Vec::new(),

            #[cfg(feature = "metrics")]
            time_of_oldest_change: None,
        }
    }
}

impl<C> Drop for Changes<C> {
    fn drop(&mut self) {
        if !self.changes.is_empty() {
            log::error!("{} changes were not processed", self.changes.len());
        }
    }
}

impl<C> Changes<C> {
    pub fn push(&mut self, change: C) {
        #[cfg(feature = "metrics")]
        {
            if self.changes.is_empty() {
                self.time_of_oldest_change = Some(Instant::now());
            }
        }

        self.changes.push(change);
    }

    pub fn accumulate(&mut self, mut changes: Changes<C>) {
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

    pub fn release(mut self) -> Vec<C> {
        mem::take(&mut self.changes)
    }

    #[cfg(feature = "metrics")]
    pub fn time_of_oldest_change(&self) -> Option<Instant> {
        self.time_of_oldest_change
    }
}
