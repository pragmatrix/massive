//! A short-lived bundle of the scene arena and the animation clock.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while [`Scene`]s are created per view. Bundling a borrow of both here keeps the
//! clock a single-owner value: no shared ownership and no interior mutability are needed.

use anyhow::Result;
use derive_more::Deref;

use massive_animation::{AnimationContext, AnimationCoordinator};
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::Scene;

#[derive(Debug, Deref)]
pub struct Frame<'a> {
    #[deref]
    scene: &'a Scene,
    animation: &'a mut AnimationCoordinator,
}

impl AnimationContext for Frame<'_> {
    fn animation_coordinator(&mut self) -> &mut AnimationCoordinator {
        self.animation
    }
}

impl<'a> Frame<'a> {
    pub fn new(scene: &'a Scene, animation: &'a mut AnimationCoordinator) -> Self {
        Self { scene, animation }
    }

    /// The scene, borrowed for the frame's full lifetime.
    ///
    /// Use this instead of the `Deref` when the reference has to outlive a mutable use of the
    /// frame.
    pub fn scene(&self) -> &'a Scene {
        self.scene
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
