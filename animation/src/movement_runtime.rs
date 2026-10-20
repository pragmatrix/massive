//! Long-lived, type-erased animations that outlive a single frame.
//!
//! A [`Movement`] owns a value of type `T` and a closure that applies animation progress to it.
//! It is mounted into a [`MovementRuntime`] and driven across many frames, unlike the
//! frame-scoped animations created directly on a `Frame`.
//!
//! # Lifecycle
//!
//! - **Mount**: [`MovementInstance`] stores a value, apply closure, and optional completion event;
//!   [`MovementRuntime::mount`] registers it and returns a [`Movement`] handle.
//! - **Modify**: [`Movement::modify`] queues a closure that mutates the value and may start
//!   animations. The closure receives an [`AnimationAllocator`] that records the movement's
//!   `ending_time` (the latest end across all its animations). The runtime then applies the
//!   movement at the time it was last drawn, so its new state is drawn in the same frame without
//!   advancing it (see [`MovementRuntime::run_actions`]).
//! - **Animate**: [`MovementRuntime::apply_animations`] advances every movement with a pending
//!   `ending_time` each cycle, until the first cycle at or past that time. It then clears the
//!   `ending_time` and emits the movement's completion event, if any.
//! - **Drop**: Dropping the [`Movement`] handle queues a `Drop` action that unregisters it.
//!
//! # Concurrency model
//!
//! Actions are queued into a shared inbox (via `Arc<Mutex<Vec<_>>>`) from any thread, then drained
//! on the runtime's thread by [`MovementRuntime::run_actions`]. The runtime owns the movement
//! values, so the apply closures and the value type `T` must be `Send + Sync`. The [`Movement`]
//! handle is only an opaque identity; it never dereferences the value.
//!
//! # Completion events
//!
//! A movement's completion callback produces a type-erased event when its animations finish. The
//! runtime returns these from
//! [`MovementRuntime::apply_animations`]; the caller downcasts them to its own event type. This is
//! how a movement signals that it has stopped, so callers do not need to track animation activity
//! themselves.

use std::any::Any;
use std::cmp::max;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::{AnimationAllocator, AnimationCoordinator, AnimationProgress};

pub struct MovementRuntime {
    movements: HashMap<MovementReference, MountedMovement>,
    action_inbox: Arc<Mutex<Vec<MovementAction>>>,
    // Reused while draining the inbox so recurring actions retain their allocation capacity.
    actions: Vec<MovementAction>,
}

impl fmt::Debug for MovementRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Movements")
            .field("count", &self.movements.len())
            .finish()
    }
}

impl Default for MovementRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MovementRuntime {
    pub fn new() -> Self {
        Self {
            movements: Default::default(),
            action_inbox: Arc::new(Mutex::new(Vec::new())),
            actions: Vec::new(),
        }
    }

    /// Mount a fully configured movement instance and return its handle.
    pub fn mount<T, F>(&mut self, instance: MovementInstance<T, F>) -> Movement<T>
    where
        T: Any + Send + Sync,
        F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
    {
        let instance = Box::new(instance);
        let reference = Movement::new(&instance.value, self.action_inbox.clone());
        self.movements.insert(
            reference.instance,
            MountedMovement {
                movement: instance,
                ending_time: None,
                drawn_at: None,
            },
        );

        reference
    }

    /// Drain the action inbox and apply each queued action to its movement.
    ///
    /// Call this once per cycle, after animations have been applied, so that actions queued by
    /// completion events take effect in the same cycle.
    ///
    /// Each modified movement is applied right away, at the time it was last drawn. A modify only
    /// changes the movement's value, and only apply-animations cycles advance movements, so
    /// without this the change would reach the scene a frame late, or never: a modify that
    /// changes the value without starting an animation sets no `ending_time` and is never
    /// advanced. A frame late is visible whenever the same transaction also changes the scene
    /// directly; re-parenting switches a location's parent at once, and its transform must follow
    /// in that frame.
    ///
    /// The last drawn time, not the cycle's, because this must show the modify without moving
    /// time: event cycles are frequent and irregular, and advancing running animations to their
    /// times makes motion stutter. An animation started in this cycle has not begun at that time,
    /// so it is drawn at its start value and keeps running; unlike a snap, nothing finishes, and
    /// the movement's `ending_time` and completion event stay with the apply-animations cycles.
    ///
    /// So apply closures run after every modify, also when no animation is running, and must
    /// handle that; a redraw at an unchanged time should write through `update_if_changed` so it
    /// submits nothing.
    pub fn run_actions(&mut self, coordinator: &mut AnimationCoordinator) {
        mem::swap(&mut self.actions, &mut *self.action_inbox.lock());
        // For a movement never drawn. `None` only when an instance's teardown flushes outside a
        // cycle, and nothing is drawn after teardown.
        let cycle_time = coordinator.cycle_time();

        for action in self.actions.drain(..) {
            match action {
                MovementAction::Modify(pointer, apply) => {
                    if let Some(instance) = self.movements.get_mut(&pointer) {
                        let mut intermediate = MovementAnimationAllocator {
                            inner: coordinator,
                            ending_time: &mut instance.ending_time,
                        };
                        apply(instance.movement.as_any_mut(), &mut intermediate);
                        if let Some(time) = instance.drawn_at.or(cycle_time) {
                            instance
                                .movement
                                .apply_animations(AnimationProgress::Proceed(time));
                            instance.drawn_at = Some(time);
                        }
                    }
                }
                MovementAction::Snap(pointer) => {
                    if let Some(instance) = self.movements.get_mut(&pointer) {
                        // Snapping is deliberately local and does not emit completion events yet.
                        instance.movement.apply_animations(AnimationProgress::Snap);
                        instance.ending_time = None;
                    }
                }
                MovementAction::Drop(pointer) => {
                    self.movements.remove(&pointer);
                }
            }
        }
    }

    /// Advance all animating movements to the given instant.
    ///
    /// Returns the completion events of movements that reached their ending time this cycle. A
    /// movement stops being advanced once its `ending_time` is reached, so callers can rely on the
    /// completion event (rather than external activity tracking) to know when it has stopped.
    pub fn apply_animations(&mut self, instant: Instant) -> Vec<Box<dyn Any + Send>> {
        let mut events = Vec::new();
        for movement in self.movements.values_mut() {
            let Some(ending_time) = movement.ending_time else {
                continue;
            };

            movement
                .movement
                .apply_animations(AnimationProgress::Proceed(instant));
            movement.drawn_at = Some(instant);

            // Keep applying through the first cycle at or past the movement end, then stop.
            if instant >= ending_time {
                movement.ending_time = None;
                if let Some(event) = movement.movement.completion_event() {
                    events.push(event);
                }
            }
        }

        events
    }
}

#[must_use]
#[derive(Debug)]
pub struct Movement<T> {
    instance: MovementReference,
    actions_inbox: Arc<Mutex<Vec<MovementAction>>>,
    marker: PhantomData<fn(T)>,
}

impl<T> Movement<T> {
    fn new(instance: &T, actions_inbox: Arc<Mutex<Vec<MovementAction>>>) -> Self {
        Self {
            instance: MovementReference::new(instance),
            actions_inbox,
            marker: PhantomData,
        }
    }

    /// Queue a closure that mutates the movement's value and may start animations.
    ///
    /// The closure runs on the runtime's thread during the next [`MovementRuntime::run_actions`].
    /// Animations started here are tracked against the movement's `ending_time`.
    pub fn modify(
        &self,
        modifier: impl FnOnce(&mut T, &mut dyn AnimationAllocator) + Send + Sync + 'static,
    ) where
        T: Any + Send + Sync,
    {
        self.actions_inbox.lock().push(MovementAction::Modify(
            self.instance,
            Box::new(move |value, context| {
                let value = value
                    .downcast_mut::<T>()
                    .expect("movement reference has the wrong value type");
                modifier(value, context);
            }),
        ));
    }

    /// Queue a snap that completes the movement's animations without emitting a completion event.
    pub fn snap(&self) {
        self.actions_inbox
            .lock()
            .push(MovementAction::Snap(self.instance));
    }
}

impl<T> Drop for Movement<T> {
    fn drop(&mut self) {
        self.actions_inbox
            .lock()
            .push(MovementAction::Drop(self.instance));
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct MovementReference(*const ());

// This is an opaque identity only; movement values are never accessed through the pointer.
unsafe impl Send for MovementReference {}
unsafe impl Sync for MovementReference {}

impl MovementReference {
    fn new<T>(value: &T) -> Self {
        Self(ptr::from_ref(value).cast())
    }
}

trait AnimatableMovement: Any + Send + Sync {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send);

    fn apply_animations(&mut self, progress: AnimationProgress);

    fn completion_event(&mut self) -> Option<Box<dyn Any + Send>>;
}

struct MountedMovement {
    movement: Box<dyn AnimatableMovement>,
    ending_time: Option<Instant>,
    /// The time the movement was last applied at, so a modify can be drawn without advancing it.
    drawn_at: Option<Instant>,
}

pub struct MovementInstance<T, F> {
    value: T,
    apply_animations: F,
    completion_event: Option<Box<dyn FnMut() -> Box<dyn Any + Send> + Send + Sync>>,
}

impl<T, F> MovementInstance<T, F> {
    pub fn new(
        value: T,
        apply_animations: F,
        completion_event: Option<Box<dyn FnMut() -> Box<dyn Any + Send> + Send + Sync>>,
    ) -> Self {
        Self {
            value,
            apply_animations,
            completion_event,
        }
    }
}

impl<T, F> AnimatableMovement for MovementInstance<T, F>
where
    T: Any + Send + Sync,
    F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
{
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        &mut self.value
    }

    fn apply_animations(&mut self, progress: AnimationProgress) {
        (self.apply_animations)(&mut self.value, progress);
    }

    fn completion_event(&mut self) -> Option<Box<dyn Any + Send>> {
        self.completion_event
            .as_mut()
            .map(|completion_event| completion_event())
    }
}

/// Forwards animation allocations to the real context while tracking this movement's ending time.
struct MovementAnimationAllocator<'a> {
    inner: &'a mut dyn AnimationAllocator,
    ending_time: &'a mut Option<Instant>,
}

impl AnimationAllocator for MovementAnimationAllocator<'_> {
    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        let start = self.inner.allocate_animation_time(duration);
        let end = start + duration;
        *self.ending_time = Some(match *self.ending_time {
            Some(existing) => max(existing, end),
            None => end,
        });
        start
    }
}

type ModifyMovement =
    Box<dyn FnOnce(&mut (dyn Any + Send), &mut dyn AnimationAllocator) + Send + Sync>;

enum MovementAction {
    Modify(MovementReference, ModifyMovement),
    Snap(MovementReference),
    Drop(MovementReference),
}

impl fmt::Debug for MovementAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Modify(_, _) => formatter.write_str("Modify"),
            Self::Snap(_) => formatter.write_str("Snap"),
            Self::Drop(_) => formatter.write_str("Drop"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Animated, Interpolation};

    /// A modify reaches the scene in the flush that runs it, like a snap but without finishing
    /// or advancing anything: a plain value change is drawn although no animation advances it, a
    /// started animation is drawn at its start value and keeps running, and retargeting a running
    /// animation in a later cycle redraws it as last drawn instead of at that cycle's time.
    #[test]
    fn modified_movements_are_drawn_in_the_same_flush() {
        let mut coordinator = AnimationCoordinator::new();
        let mut runtime = MovementRuntime::new();
        let drawn = Arc::new(Mutex::new(None));
        let sink = drawn.clone();
        let movement = runtime.mount(MovementInstance::new(
            Animated::new(0.0),
            move |value: &mut Animated<f64>, progress| {
                *sink.lock() = Some(*value.proceed_with(progress));
            },
            None,
        ));
        coordinator.begin_cycle();

        movement.modify(|value, _| value.snap(1.0));
        runtime.run_actions(&mut coordinator);
        assert_eq!(*drawn.lock(), Some(1.0));

        movement.modify(|value, context| {
            value.animate_with(context, 2.0, Duration::from_secs(1), Interpolation::Linear)
        });
        runtime.run_actions(&mut coordinator);
        assert_eq!(*drawn.lock(), Some(1.0));
        assert!(
            runtime
                .movements
                .values()
                .all(|movement| movement.ending_time.is_some()),
            "the started animation keeps running"
        );

        let start = coordinator.animation_time();
        coordinator.end_cycle();
        coordinator.upgrade_to_apply_animations_cycle(start + Duration::from_millis(500));
        runtime.apply_animations(coordinator.animation_time());
        assert_eq!(*drawn.lock(), Some(1.5));
        coordinator.end_cycle();

        coordinator.upgrade_to_apply_animations_cycle(start + Duration::from_millis(700));
        movement.modify(|value, context| {
            value.animate_with(context, 3.0, Duration::from_secs(1), Interpolation::Linear)
        });
        runtime.run_actions(&mut coordinator);
        assert_eq!(*drawn.lock(), Some(1.5));
    }
}
