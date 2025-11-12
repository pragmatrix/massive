use std::time::{Duration, Instant};

use crate::AnimationCoordinator;

/// TimeScale computes durations from one update cycle to the next.
///
/// Architecture: Shouldn't this be the underlying mechanism for [`Animated`], a more fundamental
/// one?
#[derive(Debug)]
pub struct TimeScale {
    coordinator: AnimationCoordinator,
    now: Instant,
    duration_since: Duration,
}

impl TimeScale {
    pub fn new(coordinator: AnimationCoordinator) -> Self {
        let latest_tick = coordinator.current_time();
        Self {
            coordinator,
            now: latest_tick,
            duration_since: Duration::ZERO,
        }
    }

    /// Multiply with the returned value to scale another value that is relative to seconds.
    ///
    /// Returns 0 if [`TimeScale`] was created in the current update cycle.
    pub fn scale_seconds(&mut self) -> f64 {
        self.duration_passed().as_secs_f64()
    }

    /// The duration passed since the last update cycle (ZERO if the [`TimeScale`] was just
    /// generated).
    pub fn duration_passed(&mut self) -> Duration {
        // Find out if we are in a new update cycle first.
        let current_tick = self.coordinator.current_time();
        if current_tick > self.now {
            self.duration_since = current_tick - self.now;
            self.now = current_tick;
        }
        self.duration_since
    }
}
