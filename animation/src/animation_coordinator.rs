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
//!   -   In a smooth pacing situation, it is set directly to the time the vblank was observed that
//!       triggered the cycle (`upgrade_to_apply_animations_cycle`), independent of when the event
//!       was processed.
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

/// How an animation cycle ended, and so whether the next frame still has work to animate.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CycleEnd {
    /// Animations are still running.
    Animating,
    /// No animation is left to advance.
    Settled,
}

#[derive(Debug)]
pub struct AnimationCoordinator {
    /// This is the public state that indicates if there are currently animations running.
    animating: bool,

    /// The current event processing cycle we are in.
    cycle: Option<AnimationCycle>,

    /// The time when all animations ended or will end.
    ending_time: Instant,

    /// The start time of the most recent cycle. Animation time never runs backwards past it.
    last_start_time: Instant,
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
            last_start_time: Instant::now(),
        }
    }

    /// Upgrade the current cycle to an apply animations cycle.
    ///
    /// If the cycle has not been started yet, it's started now.
    ///
    /// Only in an `ApplyAnimations` triggered cycle can we stop animations. This is so that at
    /// least one `ApplyAnimations` is running at a time > the ending time of all animations to
    /// guarantee that all the computed values represent their final values.
    ///
    /// The cycle's animation time becomes `vblank_time`, the time the vblank that triggered this
    /// cycle was observed, so that it does not depend on when the event was processed. If the cycle
    /// was already started (and so may have allocated animations), its start is moved to
    /// `vblank_time`. The time never runs backwards: it is clamped to the start of the previous
    /// cycle.
    pub fn upgrade_to_apply_animations_cycle(&mut self, vblank_time: Instant) {
        self.begin_cycle();
        let start_time = self.monotonic_start_time(vblank_time);
        let cycle = self.cycle_mut();
        cycle.start_time = start_time;
        cycle.mode = CycleMode::ApplyAnimations;
    }

    /// `true` if the current cycle is an apply-animations cycle.
    pub fn is_apply_animations_cycle(&self) -> bool {
        self.cycle
            .as_ref()
            .is_some_and(|cycle| cycle.mode == CycleMode::ApplyAnimations)
    }

    /// Start the current event processing cycle, if it has not started yet.
    pub fn begin_cycle(&mut self) {
        if self.cycle.is_none() {
            let start_time = self.monotonic_start_time(Instant::now());
            self.cycle = Some(AnimationCycle::implicit(start_time));
        }
    }

    /// Ends an update cycle and reports how it ended. This resets the current time.
    pub fn end_cycle(&mut self) -> CycleEnd {
        if let Some(cycle) = self.cycle.take() {
            if cycle.mode == CycleMode::ApplyAnimations && cycle.start_time >= self.ending_time {
                self.animating = false;
            }
        }

        if self.animating {
            CycleEnd::Animating
        } else {
            CycleEnd::Settled
        }
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

    /// Clamp a requested cycle start time to the start of the previous cycle, so that animation
    /// time never runs backwards, and record it as the most recent start.
    fn monotonic_start_time(&mut self, requested: Instant) -> Instant {
        let start_time = max(requested, self.last_start_time);
        self.last_start_time = start_time;
        start_time
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_animations_cycle_starts_at_the_vblank_time() {
        let mut coordinator = AnimationCoordinator::new();
        let vblank_time = Instant::now() + Duration::from_millis(5);
        coordinator.upgrade_to_apply_animations_cycle(vblank_time);
        assert_eq!(coordinator.animation_time(), vblank_time);
        assert!(coordinator.is_apply_animations_cycle());
    }

    #[test]
    fn upgrading_a_started_cycle_moves_its_start() {
        let mut coordinator = AnimationCoordinator::new();
        coordinator.begin_cycle();
        let vblank_time = Instant::now() + Duration::from_millis(5);
        coordinator.upgrade_to_apply_animations_cycle(vblank_time);
        assert_eq!(coordinator.animation_time(), vblank_time);
    }

    #[test]
    fn animation_time_never_runs_backwards() {
        let mut coordinator = AnimationCoordinator::new();
        let later = Instant::now() + Duration::from_millis(20);
        coordinator.upgrade_to_apply_animations_cycle(later);
        coordinator.end_cycle();

        // A vblank time older than the previous cycle's start is clamped.
        coordinator.upgrade_to_apply_animations_cycle(later - Duration::from_millis(10));
        assert_eq!(coordinator.animation_time(), later);
    }

    #[test]
    fn implicit_cycle_does_not_start_before_the_previous_cycle() {
        let mut coordinator = AnimationCoordinator::new();
        let later = Instant::now() + Duration::from_millis(50);
        coordinator.upgrade_to_apply_animations_cycle(later);
        coordinator.end_cycle();

        coordinator.begin_cycle();
        assert!(coordinator.animation_time() >= later);
    }
}
