use std::sync::{Arc, Mutex};

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
    /// Was there a request for an animation tick in this animation cycle?
    animation_ticks_requested: bool,
}

impl Tickery {
    pub fn new(now: Instant) -> Self {
        Self {
            inner: TickeryInner {
                tick: now,
                animation_ticks_requested: false,
            }
            .into(),
        }
    }

    pub fn timeline<T: Interpolatable + Send>(self: &Arc<Self>, value: T) -> Timeline<T> {
        Timeline::new(self.clone(), value)
    }

    pub fn begin_update_cycle(&self, instant: Instant) {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.tick = instant;
    }

    /// Beings an animation cycle.
    ///
    /// This sets the current tick and resets the usage count.
    ///
    /// Not &mut self, because it must be usable behing an Arc and we don't put the whole Tickery in a Mutex.
    pub fn begin_animation_cycle(&self, instant: Instant) {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.tick = instant;
        inner.animation_ticks_requested = false
    }

    /// Marks the current tick as an animation tick on and returns it.
    pub fn animation_tick(&self) -> Instant {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.animation_ticks_requested = true;
        inner.tick
    }

    /// Were there any users of the tick value since [`Self::update_tick`] was called.
    pub fn animation_ticks_requested(&self) -> bool {
        self.inner
            .lock()
            .expect("poisoned")
            .animation_ticks_requested
    }
}
