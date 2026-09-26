//! The task's animation world: the frame-cycle protocol and movement mounting.
//!
//! Crate-only on purpose (ADR 0008): animation access is gated on a live frame, so the witness
//! that carries the gate must not be reachable by consumers.

use std::any::Any;
use std::panic::Location;
use std::time::{Duration, Instant};

use massive_animation::{
    AnimationCoordinator, AnimationProgress, CycleEnd, Movement, MovementInstance, MovementRuntime,
};

use super::ANIMATION;

/// The task's animation world, gated by the frame witness.
#[derive(Debug)]
pub struct AnimationState {
    coordinator: AnimationCoordinator,
    movement: MovementRuntime,
    /// Public so [`Frame`](crate::Frame) releases its own witness under the borrow it already
    /// holds; the crate-only module keeps it out of consumer reach.
    pub witness: Option<FrameWitness>,
}

impl AnimationState {
    pub fn new(coordinator: AnimationCoordinator, movement: MovementRuntime) -> Self {
        Self {
            coordinator,
            movement,
            witness: None,
        }
    }

    pub fn assert_frame_live(&self) {
        if self.witness.is_none() {
            panic!(
                "animation state is only accessible inside a frame: no frame is live, \
                 call begin_frame() first"
            );
        }
    }

    /// Flush queued movement actions, then close the cycle and report how it ended.
    ///
    /// Actions are flushed first: completion events arrive during apply-animations cycles and may
    /// queue successor actions, which must not wait for an unrelated event.
    pub fn flush_and_end_cycle(&mut self) -> CycleEnd {
        self.movement.run_actions(&mut self.coordinator);
        self.coordinator.end_cycle()
    }
}

/// Proof that a frame is live and the task's animation state may be accessed.
///
/// Records the creation site, so the panic for a second live `begin_frame()` names both frames.
#[derive(Debug)]
pub struct FrameWitness {
    created_at: &'static Location<'static>,
}

/// Mutably borrow the task's animation state, panicking on re-entry.
pub fn with_animation_state<R>(f: impl FnOnce(&mut AnimationState) -> R) -> R {
    ANIMATION.with(|state| {
        let mut state = state
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context animation state was re-entered"));
        f(&mut state)
    })
}

/// Mutably access the current task's animation coordinator. Requires a live frame.
///
/// The frame witness is the only gate on animation state, so this panics without one.
/// [`end_frame_cycle_detached`] and [`with_animation_and_movement`] are the frame-free alternatives.
pub fn with_frame_animation<R>(f: impl FnOnce(&mut AnimationCoordinator) -> R) -> R {
    with_animation_state(|state| {
        state.assert_frame_live();
        f(&mut state.coordinator)
    })
}

/// The current frame's animation timestamp: the cycle's start time.
pub fn animation_time() -> Instant {
    with_frame_animation(|animation| animation.animation_time())
}

/// Allocate an animation duration on the current frame's clock and return its start time.
pub fn allocate_animation_time(duration: Duration) -> Instant {
    with_frame_animation(|animation| animation.allocate_animation_time(duration))
}

/// Mutably access the current task's animation coordinator and movement runtime together, without
/// requiring a live frame.
pub fn with_animation_and_movement<R>(
    f: impl FnOnce(&mut AnimationCoordinator, &mut MovementRuntime) -> R,
) -> R {
    with_animation_state(|state| f(&mut state.coordinator, &mut state.movement))
}

/// End the current frame's animation cycle and report how it ended.
pub fn end_frame_cycle() -> CycleEnd {
    with_animation_state(|state| {
        state.assert_frame_live();
        state.flush_and_end_cycle()
    })
}

/// [`end_frame_cycle`] for instance teardown, which runs after the run loop returned and so has
/// no frame.
pub fn end_frame_cycle_detached() -> CycleEnd {
    with_animation_state(AnimationState::flush_and_end_cycle)
}

/// Acquire the frame witness and begin the animation cycle: the one opener of a frame.
///
/// Returns where the blocking frame was begun if one is already live.
pub fn begin_frame_cycle(
    created_at: &'static Location<'static>,
) -> Result<(), &'static Location<'static>> {
    with_animation_state(|state| {
        if let Some(witness) = &state.witness {
            return Err(witness.created_at);
        }
        state.witness = Some(FrameWitness { created_at });
        state.coordinator.begin_cycle();
        Ok(())
    })
}

pub fn movement<T, F>(value: T, apply_animations: F) -> TaskMovementBuilder<T, F>
where
    T: Any + Send + Sync,
    F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
{
    TaskMovementBuilder {
        value,
        apply_animations,
        completion_event: None,
    }
}

/// Configuration for a movement that has not yet been mounted.
#[must_use]
pub struct TaskMovementBuilder<T, F> {
    value: T,
    apply_animations: F,
    completion_event: Option<Box<dyn FnMut() -> Box<dyn Any + Send> + Send + Sync + 'static>>,
}

impl<T, F> TaskMovementBuilder<T, F> {
    pub fn completion_event<E, G>(mut self, mut completion_event: G) -> Self
    where
        E: Any + Send,
        G: FnMut() -> E + Send + Sync + 'static,
    {
        self.completion_event = Some(Box::new(move || Box::new(completion_event())));
        self
    }
}

impl<T, F> TaskMovementBuilder<T, F>
where
    T: Any + Send + Sync,
    F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
{
    pub fn mount(self) -> Movement<T> {
        let Self {
            value,
            apply_animations,
            completion_event,
        } = self;

        // Mounting is the one ungated animation access: it happens at presenter construction,
        // before any frame exists, and touches only the movement runtime's action inbox.
        with_animation_state(|state| {
            state.movement.mount(MovementInstance::new(
                value,
                apply_animations,
                completion_event,
            ))
        })
    }
}
