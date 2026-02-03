//! A coordinating instance that is referred to by all [`Animated`] values and in the [`Scene`].
//!
//! This has two roles:
//!
//! - Provide the approximate timestamp of the next presentation to the animated values.
//! - Track which animations are currently active: It does that by recording the ending time of all
//!   animations currently active.
//!
//!   Robustness: This could be implemented by a kind of activity counter. But as of now this is
//!   just the ending timestamp of the animation that runs the longest.
//!
//!   The strategy for deciding about the current timestamp is as follows:
//!   - The current timestamp is not set initially.
//!   - The current timestamp is lazily set on first used.
//!   -   In a smooth pacing situation, it may be set earlier directly at the time the current frame
//!       was presented.
//!   - The current timestamp is reset at the time the changes are pushed to the renderer.
//!
//! # ADR Log
//!   - 20251126: Introduced two cycle modes. One implicit, and one upgraded to apply animations.
//!     This way the animation controller can clearly decide at the end of a cycle if there are
//!     animations active or not.

//!   - 202511: Decided to switch to the new model of just tracking the ending time, because
//!     deciding based on polling the value() about the render pacing felt too brittle. We don't
//!     want to a client to constrain when it is recommended to update derived values from animated
//!     values. This should be possible on every time and there should be no decision if that
//!     happens at all. Clients may just skip frames for updates, etc, which now won't cause to flip
//!     render pacing. This also has the drawback that even if animated values are active, but not
//!     actually used, the fast render pacing will stay until the animation actually end. But this
//!     is tolerable and probably won't happen in practice and should be simple to debug.

use std::{
    cmp::max,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::{Animated, Interpolatable, TimeScale};

#[derive(Debug, Clone)]
pub struct AnimationCoordinator {
    inner: Arc<Mutex<Inner>>,
}

impl Default for AnimationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationCoordinator {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                animating: false,
                cycle: None,
                ending_time: Instant::now(),
            })
            .into(),
        }
    }

    pub fn animated<T: Interpolatable + Send>(&self, value: T) -> Animated<T> {
        Animated::new(self.clone(), value)
    }

    pub fn time_scale(&self) -> TimeScale {
        TimeScale::new(self.clone())
    }

    /// Upgrade the current cycle to an apply animations cycle.
    ///
    /// If the cycle has not been started yet, it's started now.
    pub fn upgrade_to_apply_animations_cycle(&self) {
        let mut inner = self.inner.lock();
        // Be sure there is a current cycle.
        let cycle = inner.current_cycle();
        cycle.mode = CycleMode::ApplyAnimations;
    }

    /// Ends an update cycle. Returns true if animations are active. This resets the current time.
    pub fn end_cycle(&self) -> bool {
        let mut inner = self.inner.lock();
        if let Some(cycle) = inner.cycle.take() {
            if cycle.mode == CycleMode::ApplyAnimations && cycle.time >= inner.ending_time {
                inner.animating = false;
            }
        }

        inner.animating
    }

    /// Returns the current cycle time that should be used for animated values.
    ///
    /// If not set, the now is set and the cycle mode is set to implicit.
    pub(crate) fn current_cycle_time(&self) -> Instant {
        self.inner.lock().current_cycle().time
    }

    /// Allocate an animation range for the given duration and return it's starting time.
    pub(crate) fn allocate_animation_time(&self, duration: Duration) -> Instant {
        let mut inner = self.inner.lock();
        let current = inner.current_cycle().time;
        let end = current + duration;
        inner.notify_ending_time(end);
        current
    }
}

#[derive(Debug)]
struct Inner {
    /// This is the public state that indicates if there are currently animations running.
    animating: bool,

    /// The current event processing cycle we are in.
    cycle: Option<AnimationCycle>,

    /// The time when all animations end.
    ending_time: Instant,
}

impl Inner {
    fn current_cycle(&mut self) -> &mut AnimationCycle {
        self.cycle
            .get_or_insert_with(|| AnimationCycle::new(Instant::now()))
    }

    fn notify_ending_time(&mut self, ending_time: Instant) {
        self.ending_time = max(self.ending_time, ending_time);
        self.animating = true;
    }
}

#[derive(Debug, Copy, Clone)]
struct AnimationCycle {
    time: Instant,
    mode: CycleMode,
}

impl AnimationCycle {
    fn new(time: Instant) -> Self {
        Self {
            time,
            mode: CycleMode::Implicit,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CycleMode {
    #[default]
    Implicit,
    ApplyAnimations,
}
