//! Task-local access to the contexts owned by one UI task.

use std::any::Any;
use std::cell::RefCell;
use std::future::Future;

use massive_animation::{
    AnimationCoordinator, AnimationProgress, Movement, MovementInstance, MovementRuntime,
};
use massive_renderer::FontManager;
use massive_scene::{Handle, Object, Scene, SceneChange};
use tokio::task_local;

task_local! {
    static SCENE: Scene;
    static ANIMATION: RefCell<AnimationCoordinator>;
    static MOVEMENT: RefCell<MovementRuntime>;
    static SHAPER: RefCell<FontManager>;
}

/// The contexts installed together for one UI task.
#[derive(Debug)]
pub struct TaskContext {
    scene: Scene,
    animation: AnimationCoordinator,
    movement: MovementRuntime,
}

impl TaskContext {
    pub fn new(scene: Scene, animation: AnimationCoordinator, movement: MovementRuntime) -> Self {
        Self {
            scene,
            animation,
            movement,
        }
    }
}

/// Run a future with the supplied contexts installed as task-local values.
pub async fn with_context<F: Future>(contexts: TaskContext, future: F) -> F::Output {
    SCENE
        .scope(contexts.scene, async move {
            ANIMATION
                .scope(RefCell::new(contexts.animation), async move {
                    MOVEMENT
                        .scope(RefCell::new(contexts.movement), future)
                        .await
                })
                .await
        })
        .await
}

/// Run a future with a detached font manager installed as the task's shaping scratch.
pub async fn with_shaper_context<F: Future>(font_manager: FontManager, future: F) -> F::Output {
    SHAPER.scope(RefCell::new(font_manager), future).await
}

/// Mutably access the current task's shaping scratch synchronously.
pub fn with_shaper<R>(f: impl FnOnce(&mut FontManager) -> R) -> R {
    SHAPER.with(|shaper| {
        let mut shaper = shaper
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("task_context::with_shaper() was re-entered"));
        f(&mut shaper)
    })
}

/// Enter a scene object into the current task's scene change collector.
pub fn enter<T>(value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<massive_scene::Change<T::Change>>,
{
    SCENE.with(|scene| scene.enter(value))
}

/// Push an external change into the current task's scene change collector.
pub fn push_change(change: SceneChange) {
    SCENE.with(|scene| scene.push_change(change));
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
    use std::sync::Arc;

    use massive_scene::{ChangeCollector, Scene};

    use super::*;

    fn contexts() -> TaskContext {
        TaskContext::new(
            Scene::new(Arc::new(ChangeCollector::default())),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
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
            let _transform = enter(massive_geometry::Transform::IDENTITY);
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
