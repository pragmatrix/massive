//! A short-lived bundle of one animation cycle over the task's change queue.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while the change queue is installed per task. The frame opens and closes the
//! animation cycle; the task's queue is drained at submission time, so its ownership stays
//! with the task context (ADR 0008).
//!
//! The frame carries the collected change type `C` as its only state beyond the cycle guards:
//! a render submission exists only where `C: Into<SceneChange>` (the application task's scene
//! queue), while an instance frame's submission stays an `InstanceChange` set for
//! `InstanceContext::submit`. Draining a queue into the wrong submission kind is therefore a
//! compile error.

use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;
use std::time::{Duration, Instant};

use anyhow::Result;
use log::error;

use massive_animation::AnimationAllocator;
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::SceneChange;
use massive_util::ChangeSet;

use crate::task_context;

/// The bounds a frame's change kind `C` must satisfy: it converts from [`SceneChange`] and is
/// the collected type of the task's change queue (see [`task_context::take_changes`]).
///
/// Stated once here instead of on every `Frame` impl. It is public so generic code over
/// `Frame<C>` can name it; the blanket impl covers every type that satisfies the bounds.
pub trait Change: From<SceneChange> + fmt::Debug + Send + 'static {}

impl<C> Change for C where C: From<SceneChange> + fmt::Debug + Send + 'static {}

#[derive(Debug)]
pub struct Frame<C>
where
    C: Change,
{
    submitted: bool,
    created_at: &'static Location<'static>,
    _change_type: PhantomData<C>,
}

#[derive(Debug)]
pub struct FrameSubmission<C: Change> {
    changes: ChangeSet<C>,
    pacing: RenderPacing,
}

impl<C: Change> FrameSubmission<C> {
    /// The submission-level pacing; a scene-kind-agnostic submission property.
    pub fn into_pacing(self) -> RenderPacing {
        self.pacing
    }

    pub fn into_parts(self) -> (ChangeSet<C>, RenderPacing) {
        (self.changes, self.pacing)
    }
}

impl<C: Change> FrameSubmission<C>
where
    SceneChange: From<C>,
{
    /// Render submission; only the application task's scene queue produces one,
    /// because only there `C = SceneChange`.
    pub fn into_render_submission(self) -> RenderSubmission {
        RenderSubmission::new(self.changes.map(SceneChange::from), self.pacing)
    }
}

impl<C: Change> Frame<C> {
    /// Open one animation cycle; the change kind is inferred from the submission call.
    #[track_caller]
    pub fn begin() -> Self {
        task_context::with_animation(|animation| animation.begin_cycle());

        Self {
            submitted: false,
            created_at: Location::caller(),
            _change_type: std::marker::PhantomData,
        }
    }

    pub fn upgrade_to_apply_animations_cycle(&mut self) {
        task_context::with_animation(|animation| animation.upgrade_to_apply_animations_cycle());
    }

    pub fn animation_time(&self) -> Instant {
        task_context::with_animation(|animation| animation.animation_time())
    }

    // Render all the current scene changes. Only the application task's scene queue
    // (`C = SceneChange`) has a render submission.
    pub fn render_to(self, render_target: &mut dyn RenderTarget) -> Result<()>
    where
        SceneChange: From<C>,
    {
        render_target.render(self.submission().into_render_submission())
    }

    /// End the animation cycle and drain the task's change queue into a submission.
    pub fn submission(mut self) -> FrameSubmission<C> {
        let pacing = self.end_cycle();

        FrameSubmission {
            changes: task_context::take_changes::<C>(),
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

impl<C: Change> AnimationAllocator for Frame<C> {
    fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
        task_context::with_animation(|animation| animation.allocate_animation_time(duration))
    }
}

impl<C: Change> Drop for Frame<C> {
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
