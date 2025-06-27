use std::cell::Cell;

use crate::time::Instant;

/// The central coordinator for animated values.
///
/// Two roles:
/// - Provide tick values, so that clients can access current values easier without any context.
/// - Provide activity for all animated value, so that the window system knows when animations are
///   running.
#[derive(Debug)]
pub struct Coordinator {
    any_animation_active: Cell<bool>,
    current_time: Cell<Instant>,
}

impl Coordinator {
    pub fn new(now: Instant) -> Self {
        Self {
            any_animation_active: false.into(),
            current_time: now.into(),
        }
    }

    /// Begins a current update of animation cycles. This resets the active flag and updates the
    /// current time for all animated values tied to this coordinator.
    pub fn begin_update(&self, now: Instant) {
        self.current_time.set(now);
        self.any_animation_active.set(false);
    }

    pub fn any_animation_active(&self) -> bool {
        self.any_animation_active.get()
    }

    pub fn current_time(&self) -> Instant {
        self.current_time.get()
    }

    pub fn notify_active(&self) {
        self.any_animation_active.set(true);
    }
}
