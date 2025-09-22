use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::Tickery;

/// TimeScale computes durations from one update cycle to the next.
///
/// Architecture: Shouldn't this be the underlying mechanism for [`Animated`], a more fundamental
/// one?
#[derive(Debug)]
pub struct TimeScale {
    tickery: Arc<Tickery>,
    now: Instant,
    duration_since: Duration,
}

impl TimeScale {
    pub fn new(tickery: Arc<Tickery>) -> Self {
        let latest_tick = tickery.animation_tick();
        Self {
            tickery,
            now: latest_tick,
            duration_since: Duration::ZERO,
        }
    }

    /// Scale the value with the seconds passed since the previous animation update cycle. Returns 0
    /// if [`TimeScale`] was created in the current update cycle.
    pub fn scale_seconds(&mut self, value: f64) -> f64 {
        value * self.duration_passed().as_secs_f64()
    }

    /// The duration passed since the last update cycle (ZERO if the [`TimeScale`] was just
    /// generated).
    pub fn duration_passed(&mut self) -> Duration {
        // Find out if we are in a new update cycle first.
        let current_tick = self.tickery.animation_tick();
        if current_tick > self.now {
            self.duration_since = current_tick - self.now;
            self.now = current_tick;
        }
        self.duration_since
    }
}
