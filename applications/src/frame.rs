//! A short-lived bundle of the scene arena and the animation clock.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while [`Scene`]s are created per view. Bundling a borrow of both here keeps the
//! clock a single-owner value: no shared ownership and no interior mutability are needed.

use std::any::Any;
use std::time::{Duration, Instant};

use anyhow::Result;
use derive_more::Deref;

use massive_animation::{
    AnimationContext, AnimationCoordinator, Movement, MovementRuntime,
};
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::Scene;

#[derive(Debug, Deref)]
pub struct Frame<'a> {
    #[deref]
    scene: &'a Scene,
    animation: &'a mut AnimationCoordinator,
    movement: &'a mut MovementRuntime,
}

impl AnimationContext for Frame<'_> {
    fn current_cycle_time(&mut self) -> Instant {
        self.animation.current_cycle_time()
    }

    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        self.animation.allocate_animation_time(duration)
    }
}

impl<'a> Frame<'a> {
    pub fn new(
        scene: &'a Scene,
        animation: &'a mut AnimationCoordinator,
        movement: &'a mut MovementRuntime,
    ) -> Self {
        Self {
            scene,
            animation,
            movement,
        }
    }

    /// The scene, borrowed for the frame's full lifetime.
    ///
    /// Use this instead of the `Deref` when the reference has to outlive a mutable use of the
    /// frame.
    pub fn scene(&self) -> &'a Scene {
        self.scene
    }

    pub fn movement<T, F>(&mut self, value: T, apply_animations: F) -> Movement<T>
    where
        T: Any + Send + Sync,
        F: FnMut(&mut T, &mut dyn AnimationContext) + Send + Sync + 'static,
    {
        self.movement.add(value, apply_animations)
    }

    // Render all the current scene changes.
    pub fn render_to(self, render_target: &mut dyn RenderTarget) -> Result<()> {
        render_target.render(self.submission())
    }

    /// End the animation cycle and take all changes from the scene.
    pub fn submission(self) -> RenderSubmission {
        let pacing = if self.animation.end_cycle() {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };

        RenderSubmission::new(self.scene.take_changes(), pacing)
    }
}
