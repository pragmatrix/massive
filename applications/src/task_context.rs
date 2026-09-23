//! Task-local access to the contexts owned by one UI task.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::sync::Arc;

use tokio::task_local;

use massive_animation::{
    AnimationCoordinator, AnimationProgress, Movement, MovementInstance, MovementRuntime,
};
use massive_renderer::ShapingContext;
use massive_scene::{AnyCollector, ChangeSink, SceneChange};

task_local! {
    static CHANGES: AnyCollector;
    static ANIMATION: RefCell<AnimationCoordinator>;
    static MOVEMENT: RefCell<MovementRuntime>;
    static SHAPER: RefCell<ShapingContext>;
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
pub async fn with_context<F: Future>(contexts: TaskContext, future: F) -> F::Output {
    let (changes, animation, movement, shaping_context) = contexts.into_parts();

    CHANGES
        .scope(changes, async move {
            ANIMATION
                .scope(RefCell::new(animation), async move {
                    MOVEMENT
                        .scope(RefCell::new(movement), async move {
                            SHAPER.scope(RefCell::new(shaping_context), future).await
                        })
                        .await
                })
                .await
        })
        .await
}

/// Access the current task's shaping owner synchronously.
pub fn with_shaper<R>(f: impl FnOnce(&ShapingContext) -> R) -> R {
    SHAPER.with(|shaper| f(&shaper.borrow()))
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
pub fn with_animation<R>(f: impl FnOnce(&mut AnimationCoordinator) -> R) -> R {
    ANIMATION.with(|animation| {
        let mut animation = animation
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context::with_animation() was re-entered"));
        f(&mut animation)
    })
}

/// Mutably access the current task's movement runtime synchronously.
pub fn with_movement<R>(f: impl FnOnce(&mut MovementRuntime) -> R) -> R {
    MOVEMENT.with(|movement| {
        let mut movement = movement
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context::with_movement() was re-entered"));
        f(&mut movement)
    })
}

/// Mutably access the current task's animation coordinator and movement runtime together.
pub fn with_animation_and_movement<R>(
    f: impl FnOnce(&mut AnimationCoordinator, &mut MovementRuntime) -> R,
) -> R {
    ANIMATION.with(|animation| {
        let mut animation = animation
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context::with_animation() was re-entered"));
        MOVEMENT.with(|movement| {
            let mut movement = movement
                .try_borrow_mut()
                .unwrap_or_else(|_| panic!("task_context::with_movement() was re-entered"));
            f(&mut animation, &mut movement)
        })
    })
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
    async fn nested_contexts_restore_the_outer_context() {
        with_context(contexts(), async {
            with_animation(|animation| {
                animation.begin_cycle();
                animation.upgrade_to_apply_animations_cycle();
            });
            with_context(contexts(), async {
                assert!(!with_animation(|animation| {
                    animation.begin_cycle();
                    animation.is_apply_animations_cycle()
                }));
            })
            .await;
            assert!(with_animation(
                |animation| animation.is_apply_animations_cycle()
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn sibling_tasks_do_not_share_mutable_contexts() {
        with_context(contexts(), async {
            let first = tokio::spawn(with_context(contexts(), async {
                with_animation(|animation| {
                    animation.begin_cycle();
                    animation.upgrade_to_apply_animations_cycle();
                    animation.is_apply_animations_cycle()
                })
            }));
            let second = tokio::spawn(with_context(contexts(), async {
                with_animation(|animation| {
                    animation.begin_cycle();
                    animation.is_apply_animations_cycle()
                })
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
            with_animation(|_| {
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_animation(|_| ())
                }));
                assert!(panic.is_err());
            });
        })
        .await;
    }

    fn assert_send<T: Send>() {}

    #[test]
    fn contexts_are_send() {
        assert_send::<TaskContext>();
    }
}
