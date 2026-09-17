//! A short-lived bundle of the scene arena and the animation clock.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while [`Scene`]s are created per view. Bundling a borrow of both here keeps the
//! clock a single-owner value: no shared ownership and no interior mutability are needed.

use std::panic::Location;
use std::time::{Duration, Instant};

use anyhow::Result;
use derive_more::Deref;
use log::error;

use massive_animation::AnimationAllocator;
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::Scene;

use crate::task_context;

#[derive(Debug, Deref)]
pub struct Frame<'scene> {
    #[deref]
    scene: &'scene Scene,
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

impl AnimationAllocator for Frame<'_> {
    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        task_context::with_animation(|animation| animation.allocate_animation_time(duration))
    }
}

impl<'scene> Frame<'scene> {
    #[track_caller]
    pub fn new(scene: &'scene Scene) -> Self {
        task_context::with_animation(|animation| animation.begin_cycle());

        Self {
            scene,
            submitted: false,
            created_at: Location::caller(),
        }
    }

    pub fn upgrade_to_apply_animations_cycle(&mut self) {
        task_context::with_animation(|animation| animation.upgrade_to_apply_animations_cycle());
    }

    pub fn animation_time(&self) -> Instant {
        task_context::with_animation(|animation| animation.animation_time())
    }

    /// The scene, borrowed for the frame's full lifetime.
    ///
    /// Use this instead of the `Deref` when the reference has to outlive a mutable use of the
    /// frame.
    pub fn scene(&self) -> &'scene Scene {
        self.scene
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

        // Completion events arrive during apply-animation cycles and may queue successor actions.
        // Drain them now so they do not wait for unrelated input.
        task_context::with_animation_and_movement(|animation, movement| {
            movement.run_actions(animation);
        });

        if task_context::with_animation(|animation| animation.end_cycle()) {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        }
    }
}

impl Drop for Frame<'_> {
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
