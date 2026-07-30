//! A short-lived bundle of the scene arena and the animation clock.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while [`Scene`]s are created per view. Bundling a borrow of both here keeps the
//! clock a single-owner value: no shared ownership and no interior mutability are needed.

use std::any::Any;
use std::panic::Location;
use std::time::{Duration, Instant};

use anyhow::Result;
use derive_more::Deref;
use log::error;

use massive_animation::{AnimationContext, AnimationCoordinator, Movement, MovementRuntime};
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::Scene;

#[derive(Debug, Deref)]
pub struct Frame<'scene, 'context> {
    #[deref]
    scene: &'scene Scene,
    animation: &'context mut AnimationCoordinator,
    movement: &'context mut MovementRuntime,
    submitted: bool,
    created_at: &'static Location<'static>,
}

#[derive(Debug)]
pub struct FrameSubmission<'a> {
    scene: &'a Scene,
    pacing: RenderPacing,
}

impl FrameSubmission<'_> {
    pub fn render_submission(self) -> RenderSubmission {
        RenderSubmission::new(self.scene.take_changes(), self.pacing)
    }

    pub fn pacing(self) -> RenderPacing {
        self.pacing
    }
}

impl AnimationContext for Frame<'_, '_> {
    fn current_cycle_time(&self) -> Instant {
        self.animation.current_cycle_time()
    }

    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        self.animation.allocate_animation_time(duration)
    }
}

impl<'scene, 'context> Frame<'scene, 'context> {
    #[track_caller]
    pub fn new(
        scene: &'scene Scene,
        animation: &'context mut AnimationCoordinator,
        movement: &'context mut MovementRuntime,
    ) -> Self {
        animation.begin_cycle();

        Self {
            scene,
            animation,
            movement,
            submitted: false,
            created_at: Location::caller(),
        }
    }

    pub fn upgrade_to_apply_animations_cycle(&mut self) {
        self.animation.upgrade_to_apply_animations_cycle();
    }

    /// The scene, borrowed for the frame's full lifetime.
    ///
    /// Use this instead of the `Deref` when the reference has to outlive a mutable use of the
    /// frame.
    pub fn scene(&self) -> &'scene Scene {
        self.scene
    }

    pub fn movement<T, F, E, G>(
        &mut self,
        value: T,
        apply_animations: F,
        completion_event: G,
    ) -> Movement<T>
    where
        T: Any + Send + Sync,
        F: FnMut(&mut T, &dyn AnimationContext) + Send + Sync + 'static,
        E: Any + Send,
        G: FnMut() -> E + Send + Sync + 'static,
    {
        self.movement
            .mount(value, apply_animations, completion_event)
    }

    // Render all the current scene changes.
    pub fn render_to(self, render_target: &mut dyn RenderTarget) -> Result<()> {
        render_target.render(self.submission().render_submission())
    }

    /// End the animation cycle and produce its submission.
    pub fn submission(mut self) -> FrameSubmission<'scene> {
        let pacing = self.end_cycle();

        FrameSubmission {
            scene: self.scene,
            pacing,
        }
    }

    fn end_cycle(&mut self) -> RenderPacing {
        self.submitted = true;

        // Event cycles drain queued movement actions. Apply cycles intentionally do not.
        if !self.animation.is_apply_animations_cycle() {
            self.movement.run_actions(self.animation);
        }

        if self.animation.end_cycle() {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        }
    }
}

impl Drop for Frame<'_, '_> {
    fn drop(&mut self) {
        if !self.submitted {
            error!(
                "Frame was dropped without being submitted: {}:{}:{}",
                self.created_at.file(),
                self.created_at.line(),
                self.created_at.column(),
            );
        }
    }
}
