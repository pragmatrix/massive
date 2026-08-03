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
//!     happens at all. Clients may just skip frames for updates, etc., which now won't cause to
//!     flip render pacing. This also has the drawback that even if animated values are active, but
//!     not actually used, the fast render pacing will stay until the animation actually end. But
//!     this is tolerable and probably won't happen in practice and should be simple to debug.

use std::cmp::max;
use std::time::{Duration, Instant};

use crate::AnimationAllocator;

#[derive(Debug)]
pub struct AnimationCoordinator {
    /// This is the public state that indicates if there are currently animations running.
    animating: bool,

    /// The current event processing cycle we are in.
    cycle: Option<AnimationCycle>,

    /// The time when all animations ended or will end.
    ending_time: Instant,
}

impl Default for AnimationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationCoordinator {
    pub fn new() -> Self {
        Self {
            animating: false,
            cycle: None,
            ending_time: Instant::now(),
        }
    }

    /// Start the current event processing cycle, if it has not started yet.
    pub fn begin_cycle(&mut self) {
        self.cycle
            .get_or_insert_with(|| AnimationCycle::implicit(Instant::now()));
    }

    /// Upgrade the current cycle to an apply animations cycle.
    ///
    /// If the cycle has not been started yet, it's started now.
    ///
    /// Only in an `ApplyAnimations` triggered cycle can we stop animations. This is so that at
    /// least one `ApplyAnimations` is running at a time > the ending time of all animations to
    /// guarantee that all the computed values represent their final values.
    pub fn upgrade_to_apply_animations_cycle(&mut self) {
        self.begin_cycle();
        self.cycle_mut().mode = CycleMode::ApplyAnimations;
    }

    /// `true` if the current cycle is an apply-animations cycle.
    pub fn is_apply_animations_cycle(&self) -> bool {
        self.cycle
            .as_ref()
            .is_some_and(|cycle| cycle.mode == CycleMode::ApplyAnimations)
    }

    /// `true` if there are active animations right now.
    pub fn animations_active(&self) -> bool {
        self.animating
    }

    /// Ends an update cycle. Returns true if animations are active. This resets the current time.
    pub fn end_cycle(&mut self) -> bool {
        if let Some(cycle) = self.cycle.take() {
            if cycle.mode == CycleMode::ApplyAnimations && cycle.start_time >= self.ending_time {
                self.animating = false;
            }
        }

        self.animating
    }

    /// Returns the timestamp that should be used for animated values.
    pub fn animation_time(&self) -> Instant {
        self.cycle().start_time
    }

    /// Allocate an animation range for the given duration and return its starting time.
    pub fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        let current = self.cycle().start_time;
        self.notify_ending_time(current + duration);
        current
    }

    fn cycle(&self) -> &AnimationCycle {
        self.cycle
            .as_ref()
            .expect("animation cycle must be started before it is used")
    }

    fn cycle_mut(&mut self) -> &mut AnimationCycle {
        self.cycle
            .as_mut()
            .expect("animation cycle must be started before it is used")
    }

    fn notify_ending_time(&mut self, ending_time: Instant) {
        self.ending_time = max(self.ending_time, ending_time);
        self.animating = true;
    }
}

impl AnimationAllocator for AnimationCoordinator {
    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        AnimationCoordinator::allocate_animation_time(self, duration)
    }
}

#[derive(Debug, Copy, Clone)]
struct AnimationCycle {
    start_time: Instant,
    mode: CycleMode,
}

impl AnimationCycle {
    fn implicit(start_time: Instant) -> Self {
        Self {
            start_time,
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
