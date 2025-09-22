use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::{Animated, Interpolatable, TimeScale};

#[derive(Debug)]
pub struct Tickery {
    inner: Mutex<TickeryInner>,
}

#[derive(Debug)]
struct TickeryInner {
    /// The current starting time of the most recent update cycle.
    update_cycle_reference_time: Instant,

    /// Was there a request for an animation tick in this animation cycle?
    animation_ticks_requested: bool,
}

impl Tickery {
    pub fn new(now: Instant) -> Self {
        Self {
            inner: TickeryInner {
                update_cycle_reference_time: now,
                animation_ticks_requested: false,
            }
            .into(),
        }
    }

    pub fn animated<T: Interpolatable + Send>(self: &Arc<Self>, value: T) -> Animated<T> {
        Animated::new(self.clone(), value)
    }

    pub fn time_scale(self: &Arc<Self>) -> TimeScale {
        TimeScale::new(self.clone())
    }

    /// Beings an update cycle.
    ///
    /// This sets the current tick and - if this is an animation update cycle - resets the usage
    /// count.
    ///
    /// Not `&mut self`, because it must be usable behind an `Arc` and we don't put the whole
    /// `Tickery` in a `Mutex`.
    pub fn begin_update_cycle(&self, instant: Instant, animation_cycle: bool) {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.update_cycle_reference_time = instant;
        if animation_cycle {
            inner.animation_ticks_requested = false;
        }
    }

    /// Were there any users of the tick value since [`Self::update_tick`] was called or any active
    /// animation tokens.
    ///
    /// See [`AnimationToken`] and [`named_token()`].
    pub fn animation_ticks_needed(&self) -> bool {
        self.inner
            .lock()
            .expect("poisoned")
            .animation_ticks_requested
    }

    /// Marks the current tick as an animation tick on and returns it.
    pub fn animation_tick(&self) -> Instant {
        let mut inner = self.inner.lock().expect("poisoned");
        inner.animation_ticks_requested = true;
        inner.update_cycle_reference_time
    }

    /// Were there any users of the tick value since [`Self::update_tick`] was called.
    pub fn animation_ticks_requested(&self) -> bool {
        self.inner
            .lock()
            .expect("poisoned")
            .animation_ticks_requested
    }
}
