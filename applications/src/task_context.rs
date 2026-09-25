//! Task-local access to the contexts owned by one UI task.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::panic::Location;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task_local;

use massive_animation::{
    AnimationCoordinator, AnimationProgress, Movement, MovementInstance, MovementRuntime,
};
use massive_renderer::{FontManager, ShapingContext};
use massive_scene::{AnyCollector, ChangeSink, SceneChange};

task_local! {
    static CHANGES: AnyCollector;
    static ANIMATION: RefCell<AnimationState>;
    static SHAPER: RefCell<ShapingContext>;
}

/// The task's animation world, gated by the frame witness.
///
/// One cell rather than three: the coordinator, the movement runtime, and the witness are
/// accessed together (each `run_actions`/`apply_animations` step touches both runtimes, and the
/// witness gates access to both), so separate cells only added nested borrows and a second
/// occupancy check per access. Movement is also reached on its own — mounting happens before any
/// frame exists — through the ungated [`with_movement`].
#[derive(Debug)]
pub(crate) struct AnimationState {
    coordinator: AnimationCoordinator,
    movement: MovementRuntime,
    witness: Option<FrameWitness>,
}

impl AnimationState {
    fn new(coordinator: AnimationCoordinator, movement: MovementRuntime) -> Self {
        Self {
            coordinator,
            movement,
            witness: None,
        }
    }

    /// Panic unless a frame is live: the witness is the only gate on animation state.
    fn assert_frame_live(&self) {
        if self.witness.is_none() {
            panic!(
                "animation state is only accessible inside a frame: no frame is live, \
                 call begin_frame() first"
            );
        }
    }
}

/// Proof that a frame is live and the task's animation state may be accessed.
///
/// Why the witness records the frame's creation site: occupancy alone would be enough for the
/// gate, and the owned [`Frame`](crate::Frame) carries its own diagnostics. `created_at` exists
/// for the one panic only the witness can produce — a second `begin_frame()` while a frame is
/// live. The live frame is a plain local further up the stack that the witness deliberately does
/// not own, so without the recorded site the panic could only name the second call site and the
/// culprit would have to be recovered from a backtrace. Recording where the blocking frame was
/// begun names both frames unconditionally.
#[derive(Debug)]
pub(crate) struct FrameWitness {
    created_at: &'static Location<'static>,
}

/// The contexts installed together for one UI task.
#[derive(Debug)]
pub struct TaskContext {
    changes: AnyCollector,
    animation: AnimationCoordinator,
    movement: MovementRuntime,
    shaping_context: ShapingContext,
}

impl TaskContext {
    pub fn new(
        changes: AnyCollector,
        animation: AnimationCoordinator,
        movement: MovementRuntime,
        shaping_context: ShapingContext,
    ) -> Self {
        Self {
            changes,
            animation,
            movement,
            shaping_context,
        }
    }

    /// The task's change queue, moved into the task-local scope by `with_context`.
    fn into_parts(
        self,
    ) -> (
        AnyCollector,
        AnimationCoordinator,
        MovementRuntime,
        ShapingContext,
    ) {
        (
            self.changes,
            self.animation,
            self.movement,
            self.shaping_context,
        )
    }
}

/// Run a future with the supplied contexts installed as task-local values.
async fn with_all_contexts<F: Future>(
    changes: AnyCollector,
    animation_state: RefCell<AnimationState>,
    shaping_context: RefCell<ShapingContext>,
    future: F,
) -> F::Output {
    CHANGES
        .scope(changes, async move {
            ANIMATION
                .scope(animation_state, async move {
                    SHAPER.scope(shaping_context, future).await
                })
                .await
        })
        .await
}

/// Run a future with the supplied contexts installed as task-local values.
pub async fn with_context<F: Future>(contexts: TaskContext, future: F) -> F::Output {
    let (changes, animation, movement, shaping_context) = contexts.into_parts();
    with_all_contexts(
        changes,
        RefCell::new(AnimationState::new(animation, movement)),
        RefCell::new(shaping_context),
        future,
    )
    .await
}

/// Access the current task's shaping owner synchronously.
pub fn with_shaper<R>(f: impl FnOnce(&ShapingContext) -> R) -> R {
    SHAPER.with(|shaper| f(&shaper.borrow()))
}

/// The font manager of the current task's shaping owner.
///
/// A context observes the manager's face authority and published registry rather than owning
/// fonts (ADR 0005), so this is the one identity world the task shapes and renders with — the
/// manager the shell built from the [`FontPolicy`](massive_renderer::FontPolicy) passed to
/// `shell::run`. Fonts the application needs are loaded into it.
pub fn fonts() -> FontManager {
    with_shaper(|shaping_context| shaping_context.manager())
}

/// Collect one change of the installed change type.
pub fn collect<C>(change: C)
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    CHANGES.with(|collector| collector.collect::<C>(change));
}

/// Drain all collected changes of the installed change type.
pub fn take_changes<C>() -> massive_util::ChangeSet<C>
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    CHANGES.with(|collector| collector.take_all::<C>())
}

/// Borrow the task's change queue for explicit enter/push operations.
pub fn with_changes<R>(f: impl FnOnce(&AnyCollector) -> R) -> R {
    CHANGES.with(f)
}

/// The erased sink of the task's change queue, for callers that hold it across calls
/// (long-lived visuals, builder chains) and cannot borrow the task-local collector.
pub fn sink() -> Arc<dyn ChangeSink> {
    CHANGES.with(|collector| collector.sink().clone())
}

/// Mutably access the current task's animation coordinator synchronously.
///
/// Gated: requires a live frame ([`crate::Frame`], begun via [`crate::begin_frame`]). The frame
/// witness proves animation activity is declared; without it animation state is unreachable by
/// design, so this panics. The sanctioned frame-free exception is
/// [`with_detached_animation_cycle`].
pub(crate) fn with_animation<R>(f: impl FnOnce(&mut AnimationCoordinator) -> R) -> R {
    with_animation_state(|state| {
        state.assert_frame_live();
        f(&mut state.coordinator)
    })
}

/// The current frame's animation timestamp: the cycle's start time.
pub fn animation_time() -> Instant {
    with_animation(|animation| animation.animation_time())
}

/// Allocate an animation duration on the current frame's clock and return its start time.
///
/// The ambient allocation step, read from the same gated clock as [`animation_time`]; the two are
/// the whole allocator surface the ambient API needs, so no allocator object exists for it.
pub(crate) fn allocate_animation_time(duration: Duration) -> Instant {
    with_animation(|animation| animation.allocate_animation_time(duration))
}

/// The one sanctioned animation access without a live frame: the shell's apply-animations step
/// and instance teardown.
///
/// Acquires the frame witness for the duration of `f` and releases it afterwards; the animation
/// cycle itself is intentionally not opened or closed here (the shell's step runs mid-cycle, and
/// teardown closing is `f`'s concern). If a witness is somehow already held — e.g. an
/// unsubmitted frame held across a panic unwind — it is joined and left alone instead of being
/// replaced, so this never panics and never steals another frame's witness.
pub fn with_detached_animation_cycle<R>(
    f: impl FnOnce(&mut AnimationCoordinator, &mut MovementRuntime) -> R,
) -> R {
    let acquired = acquire_witness_if_unheld();
    let result = with_animation_and_movement(|animation, movement| f(animation, movement));
    if acquired {
        release_frame_witness();
    }
    result
}

/// Mutably access the current task's movement runtime synchronously.
///
/// Deliberately ungated: mounting a movement ([`TaskMovementBuilder::mount`]) is presenter
/// construction work that runs before any frame exists. The gate lives on the other two fields
/// of the same [`AnimationState`], so this is the only way to reach movement without a frame.
pub fn with_movement<R>(f: impl FnOnce(&mut MovementRuntime) -> R) -> R {
    with_animation_state(|state| f(&mut state.movement))
}

/// Mutably access the current task's animation coordinator and movement runtime together.
pub(crate) fn with_animation_and_movement<R>(
    f: impl FnOnce(&mut AnimationCoordinator, &mut MovementRuntime) -> R,
) -> R {
    with_animation_state(|state| {
        state.assert_frame_live();
        f(&mut state.coordinator, &mut state.movement)
    })
}

/// Mutably borrow the task's animation state, panicking on re-entry.
///
/// The single borrow point for all animation state: a re-entrant access (e.g. an ambient
/// allocation from inside a movement closure, which already holds this borrow) fails loudly
/// here rather than silently observing half-updated state.
fn with_animation_state<R>(f: impl FnOnce(&mut AnimationState) -> R) -> R {
    ANIMATION.with(|state| {
        let mut state = state
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context animation state was re-entered"));
        f(&mut state)
    })
}

/// Acquire the frame witness and begin the animation cycle: the one opener of a frame.
///
/// Both steps share one borrow, so a frame is never installed without its cycle having begun.
/// Returns where the blocking frame was begun if one is already live.
pub(crate) fn begin_frame_cycle(
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

/// Acquire the witness only when none is held; returns whether this call acquired it.
///
/// The detached access never opens a cycle: the shell's span runs mid-cycle and teardown closes
/// the cycle itself. Joining an existing witness rather than replacing it keeps this a Drop-safe
/// no-op when a frame is held across a panic unwind.
fn acquire_witness_if_unheld() -> bool {
    let created_at = Location::caller();
    with_animation_state(|state| match state.witness {
        Some(_) => false,
        None => {
            state.witness = Some(FrameWitness { created_at });
            true
        }
    })
}

/// Release the frame witness.
pub(crate) fn release_frame_witness() {
    with_animation_state(|state| state.witness = None);
}

/// Build a movement that mounts into the current task context when requested.
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
    /// Attach a callback that produces an event when this movement's animations finish.
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
    /// Mount this movement in the current task context and return its typed handle.
    pub fn mount(self) -> Movement<T> {
        let Self {
            value,
            apply_animations,
            completion_event,
        } = self;

        with_movement(|runtime| {
            runtime.mount(MovementInstance::new(
                value,
                apply_animations,
                completion_event,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::{AnyCollector, SceneChange};

    use super::*;
    use crate::ambient::Enter;

    fn contexts() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<SceneChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            // Any engine compiled into this build works: the tests never shape.
            FontManager::bare(ShapingEngineKind::available()[0]).new_shaping_context(),
        )
    }

    #[tokio::test]
    async fn access_requires_an_installed_context() {
        let panic = tokio::spawn(async { with_animation(|_| ()) })
            .await
            .unwrap_err();
        assert!(panic.is_panic());
    }

    #[tokio::test]
    async fn animation_access_requires_a_live_frame() {
        with_context(contexts(), async {
            // A context is installed, but no frame is open: the gate must panic.
            let panic = tokio::spawn(async { with_animation(|_| ()) })
                .await
                .unwrap_err();
            assert!(panic.is_panic());

            // Animation access works while a frame is live.
            let frame = crate::begin_frame();
            with_animation(|animation| {
                animation.upgrade_to_apply_animations_cycle();
                assert!(animation.is_apply_animations_cycle());
            });

            // The frame's drop releases the witness; the next begin works again.
            drop(frame);
            let frame = crate::begin_frame();
            with_animation(|animation| {
                assert!(!animation.is_apply_animations_cycle());
            });
            drop(frame);
        })
        .await;
    }

    #[tokio::test]
    #[should_panic(expected = "while a frame begun at")]
    async fn double_begin_panics() {
        with_context(contexts(), async {
            let _first = crate::begin_frame();
            let _second = crate::begin_frame();
        })
        .await;
    }

    #[tokio::test]
    async fn dropped_frame_releases_the_witness() {
        with_context(contexts(), async {
            {
                let frame = crate::begin_frame();
                frame.submission::<SceneChange>();
            }
            // The unsubmitted frame's drop must release the witness, not poison it: this begin
            // must not panic.
            let frame = crate::begin_frame();
            frame.submission::<SceneChange>();
        })
        .await;
    }

    #[tokio::test]
    async fn detached_cycle_witnesses_access_without_a_frame() {
        with_context(contexts(), async {
            let result = with_detached_animation_cycle(|animation, _| {
                animation.upgrade_to_apply_animations_cycle();
                animation.animation_time()
            });
            let _ = result;

            // The detached witness is released again: animation access panics once more.
            let panic = tokio::spawn(async { with_animation(|_| ()) })
                .await
                .unwrap_err();
            assert!(panic.is_panic());

            // A live frame is joined, not replaced: the witness survives the detached access.
            let frame = crate::begin_frame();
            with_detached_animation_cycle(|animation, _| {
                assert!(animation.is_apply_animations_cycle());
            });
            drop(frame);
        })
        .await;
    }

    #[tokio::test]
    async fn nested_contexts_restore_the_outer_context() {
        with_context(contexts(), async {
            let outer_frame = crate::begin_frame();
            with_animation(|animation| {
                animation.upgrade_to_apply_animations_cycle();
            });
            with_context(contexts(), async {
                let inner_frame = crate::begin_frame();
                assert!(!with_animation(|animation| {
                    animation.is_apply_animations_cycle()
                }));
                inner_frame.submission::<SceneChange>();
            })
            .await;
            assert!(with_animation(
                |animation| animation.is_apply_animations_cycle()
            ));
            outer_frame.submission::<SceneChange>();
        })
        .await;
    }

    #[tokio::test]
    async fn sibling_tasks_do_not_share_mutable_contexts() {
        with_context(contexts(), async {
            let first = tokio::spawn(with_context(contexts(), async {
                let frame = crate::begin_frame();
                let upgraded = with_animation(|animation| {
                    animation.upgrade_to_apply_animations_cycle();
                    animation.is_apply_animations_cycle()
                });
                frame.submission::<SceneChange>();
                upgraded
            }));
            let second = tokio::spawn(with_context(contexts(), async {
                let frame = crate::begin_frame();
                let fresh = with_animation(|animation| animation.is_apply_animations_cycle());
                frame.submission::<SceneChange>();
                fresh
            }));
            assert!(first.await.unwrap());
            assert!(!second.await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn enter_uses_the_installed_scene() {
        with_context(contexts(), async {
            let _transform = massive_geometry::Transform::IDENTITY.enter();
        })
        .await;
    }

    #[tokio::test]
    async fn mutable_access_rejects_reentry() {
        with_context(contexts(), async {
            let frame = crate::begin_frame();
            with_animation(|_| {
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_animation(|_| ())
                }));
                assert!(panic.is_err());
            });
            frame.submission::<SceneChange>();
        })
        .await;
    }

    fn assert_send<T: Send>() {}

    #[test]
    fn contexts_are_send() {
        assert_send::<TaskContext>();
    }
}
