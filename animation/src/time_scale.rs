use std::time::{Duration, Instant};

use crate::AnimationContext;

/// `TimeScale` computes durations from one update cycle to the next.
///
/// Architecture: Shouldn't this be the underlying mechanism for [`Animated`], a more fundamental
/// one?
#[derive(Debug)]
pub struct TimeScale {
    now: Instant,
    duration_since: Duration,
}

impl TimeScale {
    pub fn new(context: &impl AnimationContext) -> Self {
        let current_time = context.current_cycle_time();
        Self {
            now: current_time,
            duration_since: Duration::ZERO,
        }
    }

    /// Multiply with the returned value to scale another value that is relative to seconds.
    ///
    /// Returns 0 if [`TimeScale`] was created in the current update cycle.
    pub fn scale_seconds(&mut self, context: &impl AnimationContext) -> f64 {
        self.duration_passed(context).as_secs_f64()
    }

    /// The duration passed since the last update cycle (ZERO if the [`TimeScale`] was just
    /// generated).
    pub fn duration_passed(&mut self, context: &impl AnimationContext) -> Duration {
        // Find out if we are in a new update cycle first.
        let current_time = context.current_cycle_time();
        if current_time > self.now {
            self.duration_since = current_time - self.now;
            self.now = current_time;
        }
        self.duration_since
    }
}
