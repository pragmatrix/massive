//! Long-lived, type-erased animations that outlive a single frame.
//!
//! A [`Movement`] owns a value of type `T` and a closure that applies animation progress to it.
//! It is mounted into a [`MovementRuntime`] and driven across many frames, unlike the
//! frame-scoped animations created directly on a `Frame`.
//!
//! # Lifecycle
//!
//! - **Mount**: [`MovementRuntime::movement`] wraps a value and an apply closure, then
//!   [`MovementBuilder::mount`] registers it and returns a [`Movement`] handle.
//! - **Modify**: [`Movement::modify`] queues a closure that mutates the value and may start
//!   animations. The closure receives an [`AnimationAllocator`] that records the movement's
//!   `ending_time` (the latest end across all its animations).
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
//! [`MovementBuilder::completion_event`] attaches a callback that produces a type-erased event
//! when the movement's animations finish. The runtime returns these from
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

use crate::{AnimationAllocator, AnimationProgress};

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

    /// Start building a long-lived movement from a value and its apply closure.
    ///
    /// The apply closure is called with the current animation progress each cycle while the
    /// movement is animating. Configure it with [`MovementBuilder::completion_event`] and register
    /// it with [`MovementBuilder::mount`].
    pub fn movement<T, F>(&mut self, value: T, apply_animations: F) -> MovementBuilder<'_, T, F>
    where
        T: Any + Send + Sync,
        F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
    {
        MovementBuilder {
            runtime: self,
            instance: MovementInstance {
                value,
                apply_animations,
                completion_event: None,
            },
        }
    }

    fn mount<T, F>(&mut self, instance: MovementInstance<T, F>) -> Movement<T>
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
            },
        );

        reference
    }

    /// Drain the action inbox and apply each queued action to its movement.
    ///
    /// Call this once per cycle, after animations have been applied, so that actions queued by
    /// completion events take effect in the same cycle.
    pub fn run_actions(&mut self, context: &mut dyn AnimationAllocator) {
        mem::swap(&mut self.actions, &mut *self.action_inbox.lock());

        for action in self.actions.drain(..) {
            match action {
                MovementAction::Modify(pointer, apply) => {
                    if let Some(instance) = self.movements.get_mut(&pointer) {
                        let mut intermediate = MovementAnimationAllocator {
                            inner: context,
                            ending_time: &mut instance.ending_time,
                        };
                        apply(instance.movement.as_any_mut(), &mut intermediate);
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
pub struct MovementBuilder<'a, T, F> {
    runtime: &'a mut MovementRuntime,
    instance: MovementInstance<T, F>,
}

impl<T, F> MovementBuilder<'_, T, F> {
    /// Attach a callback that produces an event when this movement's animations finish.
    ///
    /// The event is type-erased and returned from [`MovementRuntime::apply_animations`]; the
    /// caller downcasts it to its own event type.
    pub fn completion_event<E, G>(mut self, mut completion_event: G) -> Self
    where
        E: Any + Send,
        G: FnMut() -> E + Send + Sync + 'static,
    {
        self.instance.completion_event = Some(Box::new(move || Box::new(completion_event())));
        self
    }
}

impl<T, F> MovementBuilder<'_, T, F>
where
    T: Any + Send + Sync,
    F: FnMut(&mut T, AnimationProgress) + Send + Sync + 'static,
{
    /// Register the movement with the runtime and return its handle.
    pub fn mount(self) -> Movement<T> {
        self.runtime.mount(self.instance)
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
}

struct MovementInstance<T, F> {
    value: T,
    apply_animations: F,
    completion_event: Option<Box<dyn FnMut() -> Box<dyn Any + Send> + Send + Sync>>,
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
