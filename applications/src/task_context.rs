//! Task-local access to the contexts owned by one UI task.
//!
//! The task's animation world lives in the crate-only [`animation`] module; the re-exports below
//! are the consumer-facing part of it.

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::sync::Arc;

use tokio::task_local;

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_renderer::{FontManager, ShapingContext};
use massive_scene::{AnyCollector, ChangeSink, SceneChange};

pub(crate) mod animation;
mod shaper;

/// The animation accessors an application may call.
pub use animation::{TaskMovementBuilder, animation_time, movement, with_animation_and_movement};

/// The owning shaping handle an application holds while it shapes.
pub use shaper::{Shaper, shaper};

/// The frame-cycle protocol: driven by [`Frame`](crate::Frame), not by applications.
pub(crate) use animation::{
    AnimationState, allocate_animation_time, begin_frame_cycle, end_frame_cycle,
    end_frame_cycle_detached, with_animation_state, with_frame_animation,
};

task_local! {
    static CHANGES: AnyCollector;
    static ANIMATION: RefCell<AnimationState>;
    // `Option` so an owning `Shaper` can move the context out and restore it on drop;
    // `fonts()` borrows the slot in place instead, so a manager read never mints a handle.
    static SHAPER: RefCell<Option<ShapingContext>>;
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

async fn with_all_contexts<F: Future>(
    changes: AnyCollector,
    animation_state: RefCell<AnimationState>,
    shaping_context: ShapingContext,
    future: F,
) -> F::Output {
    CHANGES
        .scope(changes, async move {
            ANIMATION
                .scope(animation_state, async move {
                    SHAPER
                        .scope(RefCell::new(Some(shaping_context)), future)
                        .await
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
        shaping_context,
        future,
    )
    .await
}

/// The font manager of the current task's shaping owner: the manager the shell built from the
/// [`FontPolicy`](massive_renderer::FontPolicy) passed to `shell::run`.
///
/// Reads the manager through the installed [`ShapingContext`], its owner, without taking the
/// context out of the task-local or opening a [`Shaper`] handle.
///
/// The one constraint is exclusivity: a [`Shaper`] handle holds the context checked out for as
/// long as it is open, so reading the manager while shaping is an inherent conflict and `fonts()`
/// panics then. Install the task context with [`with_context`] before calling it, and drop any
/// open handle first.
pub fn fonts() -> FontManager {
    SHAPER
        .try_with(|slot| {
            slot.borrow()
                .as_ref()
                .unwrap_or_else(|| {
                    panic!(
                        "shaping context checked out: a Shaper handle is open in this task, so \
                         fonts() cannot read the manager while shaping; drop the handle before \
                         reading fonts()"
                    )
                })
                .manager()
        })
        .unwrap_or_else(|_| {
            panic!(
                "no shaping context installed: fonts() requires the task context, so call \
                 task_context::with_context before reading fonts()"
            )
        })
}

pub fn submit<C>(change: C)
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    with_changes(|collector| collector.collect::<C>(change));
}

pub fn take_changes<C>() -> massive_util::ChangeSet<C>
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    with_changes(|collector| collector.take_all::<C>())
}

/// The erased sink of the task's change queue, for callers that cannot borrow the task-local
/// collector across calls (long-lived visuals, builder chains).
pub fn sink() -> Arc<dyn ChangeSink> {
    with_changes(|collector| collector.sink().clone())
}

pub fn with_changes<R>(f: impl FnOnce(&AnyCollector) -> R) -> R {
    CHANGES.with(f)
}

#[cfg(test)]
mod tests {
    use massive_animation::CycleEnd;
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::{AnyCollector, SceneChange};

    use super::*;

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
        let panic = tokio::spawn(async { with_frame_animation(|_| ()) })
            .await
            .unwrap_err();
        assert!(panic.is_panic());
    }

    #[tokio::test]
    async fn animation_access_requires_a_live_frame() {
        with_context(contexts(), async {
            // A context is installed, but no frame is open: the gate must panic.
            let panic = tokio::spawn(async { with_frame_animation(|_| ()) })
                .await
                .unwrap_err();
            assert!(panic.is_panic());

            let frame = crate::begin_frame();
            with_frame_animation(|animation| {
                animation.upgrade_to_apply_animations_cycle();
                assert!(animation.is_apply_animations_cycle());
            });

            drop(frame);
            let frame = crate::begin_frame();
            with_frame_animation(|animation| {
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
    async fn detached_frame_end_closes_the_cycle_without_a_frame() {
        with_context(contexts(), async {
            // Only the detached form may close the cycle without a live frame.
            assert_eq!(end_frame_cycle_detached(), CycleEnd::Settled);

            // The gated end still refuses without a frame; its panic cannot be confused with
            // re-entry, since no borrow is held here.
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(end_frame_cycle));
            assert!(panic.is_err());

            // A live frame still ends through the gated path.
            let frame = crate::begin_frame();
            assert_eq!(end_frame_cycle_detached(), CycleEnd::Settled);
            frame.submission::<SceneChange>();
        })
        .await;
    }

    #[tokio::test]
    async fn nested_contexts_restore_the_outer_context() {
        with_context(contexts(), async {
            let outer_frame = crate::begin_frame();
            with_frame_animation(|animation| {
                animation.upgrade_to_apply_animations_cycle();
            });
            with_context(contexts(), async {
                let inner_frame = crate::begin_frame();
                assert!(!with_frame_animation(|animation| {
                    animation.is_apply_animations_cycle()
                }));
                inner_frame.submission::<SceneChange>();
            })
            .await;
            assert!(with_frame_animation(
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
                let upgraded = with_frame_animation(|animation| {
                    animation.upgrade_to_apply_animations_cycle();
                    animation.is_apply_animations_cycle()
                });
                frame.submission::<SceneChange>();
                upgraded
            }));
            let second = tokio::spawn(with_context(contexts(), async {
                let frame = crate::begin_frame();
                let fresh = with_frame_animation(|animation| animation.is_apply_animations_cycle());
                frame.submission::<SceneChange>();
                fresh
            }));
            assert!(first.await.unwrap());
            assert!(!second.await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn mutable_access_rejects_reentry() {
        with_context(contexts(), async {
            let frame = crate::begin_frame();
            with_frame_animation(|_| {
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_frame_animation(|_| ())
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
