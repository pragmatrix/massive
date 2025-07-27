use std::sync::{
    self,
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::{Interpolatable, Timeline};

#[derive(Debug)]
pub struct Tickery {
    inner: Mutex<TickeryInner>,
}

#[derive(Debug)]
struct TickeryInner {
    tick: Instant,
    any_users: bool,
}

impl Tickery {
    pub fn new(now: Instant) -> Self {
        Self {
            inner: TickeryInner {
                tick: now.into(),
                any_users: false.into(),
            }
            .into(),
        }
    }

    pub fn timeline<T: Interpolatable + Send>(self: &Arc<Self>, value: T) -> Timeline<T> {
        Timeline::new(self.clone(), value)
    }

    /// Update the current tick.
    ///
    /// This sets the current tick and resets the usage count.
    ///
    /// Not &mut self, because it must be usable behing an Arc and we don't put the whole Tickery in a Mutex.
    pub fn prepare_frame(&self, instant: Instant) {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.tick = instant;
        inner.any_users = false;
    }

    /// Marks the current tick as _used_ on and returns it.
    pub fn current_tick(&self) -> Instant {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.any_users = true;
        inner.tick
    }

    /// Were there any users of the tick value since [`Self::update_tick`] was called.
    pub fn any_users(&self) -> bool {
        self.inner.lock().expect("poisoned").any_users
    }
}
