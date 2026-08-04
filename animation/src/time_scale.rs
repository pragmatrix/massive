use std::time::{Duration, Instant};

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
    pub fn new(now: Instant) -> Self {
        Self {
            now,
            duration_since: Duration::ZERO,
        }
    }

    /// Multiply with the returned value to scale another value that is relative to seconds.
    ///
    /// Returns 0 if [`TimeScale`] was created in the current update cycle.
    pub fn scale_seconds(&mut self, now: Instant) -> f64 {
        self.duration_passed(now).as_secs_f64()
    }

    /// The duration passed since the last update cycle (ZERO if the [`TimeScale`] was just
    /// generated).
    pub fn duration_passed(&mut self, now: Instant) -> Duration {
        // Find out if we are in a new update cycle first.
        if now > self.now {
            self.duration_since = now - self.now;
            self.now = now;
        }
        self.duration_since
    }
}
