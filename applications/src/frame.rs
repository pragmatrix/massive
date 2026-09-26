//! A short-lived bundle of one animation cycle over the task's change queue.
//!
//! The [`AnimationCoordinator`] is owned by exactly one context (an instance or the application),
//! while the change queue is installed per task. The frame opens and closes the animation cycle
//! and drains the task's queue at submission time (ADR 0008). [`Frame::render_to`] and
//! [`Frame::render_submission`] expose the application task's scene queue.

use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;

use anyhow::Result;
use log::error;

use massive_animation::CycleEnd;
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::SceneChange;
use massive_util::ChangeSet;

use crate::task_context;

/// The bounds a frame's change kind `C` must satisfy, stated once so generic code over
/// [`FrameSubmission`] can name them.
pub trait Change: From<SceneChange> + fmt::Debug + Send + 'static {}

impl<C> Change for C where C: From<SceneChange> + fmt::Debug + Send + 'static {}

/// The pacing a cycle end asks of the next frame.
///
/// [`CycleEnd`] is animation vocabulary and [`RenderPacing`] render-target vocabulary, so the
/// mapping lives here, with the frame that submits under it. Instance teardown maps its detached
/// cycle end the same way: the last pacing of the instance is the pacing its final submission
/// carries.
pub(crate) fn pacing_for(cycle_end: CycleEnd) -> RenderPacing {
    match cycle_end {
        CycleEnd::Animating => RenderPacing::Smooth,
        CycleEnd::Settled => RenderPacing::Fast,
    }
}

#[derive(Debug)]
pub struct Frame {
    submitted: bool,
    created_at: &'static Location<'static>,
    // `Frame` owns its task's frame witness and releases it on drop; a witness must never be
    // released on behalf of another task, so the frame is confined to its task. The raw-pointer
    // PhantomData makes `Frame` `!Send` and `!Sync` (a `*mut ()` is neither).
    _task_local: PhantomData<*mut ()>,
}

#[derive(Debug)]
pub struct FrameSubmission<C: Change> {
    changes: ChangeSet<C>,
    pacing: RenderPacing,
}

impl<C: Change> FrameSubmission<C> {
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

/// Open one animation cycle over the task's change queue.
///
/// A task context must be installed: the frame reads the task's animation clock.
#[track_caller]
pub fn begin_frame() -> Frame {
    let created_at = Location::caller();

    // The returned site names the blocking frame so both frames appear in the panic.
    if let Err(live_at) = task_context::begin_frame_cycle(created_at) {
        panic!(
            "begin_frame() attempted at {}:{}:{} while a frame begun at {}:{}:{} is still live \
             (a frame must be submitted before the next one opens)",
            created_at.file(),
            created_at.line(),
            created_at.column(),
            live_at.file(),
            live_at.line(),
            live_at.column(),
        );
    }

    Frame {
        submitted: false,
        created_at,
        _task_local: PhantomData,
    }
}

impl Frame {
    pub fn upgrade_to_apply_animations_cycle(&mut self) {
        task_context::with_frame_animation(|animation| {
            animation.upgrade_to_apply_animations_cycle()
        });
    }

    /// Submit this frame's changes to `render_target` in one call: the application task's scene
    /// queue drained and rendered.
    pub fn render_to(self, render_target: &mut dyn RenderTarget) -> Result<()> {
        render_target.render(self.render_submission())
    }

    /// The application task's render submission.
    pub fn render_submission(self) -> RenderSubmission {
        self.submission::<SceneChange>().into_render_submission()
    }

    /// End the animation cycle and drain the task's change queue into a submission of the
    /// change kind `C`.
    pub fn submission<C: Change>(mut self) -> FrameSubmission<C> {
        let pacing = pacing_for(self.end_cycle());

        FrameSubmission {
            changes: task_context::take_changes::<C>(),
            pacing,
        }
    }

    /// Close this frame's animation cycle and report how it ended.
    fn end_cycle(&mut self) -> CycleEnd {
        self.submitted = true;
        task_context::end_frame_cycle()
    }
}

impl Drop for Frame {
    // Release-only, unconditionally: frames are legitimately dropped unsubmitted on the normal
    // quit paths (e.g. the desktop's CloseRequested return, which then opens further frames
    // during shutdown), so a missing submit must never poison the witness for the next frame —
    // nor leak this frame's open cycle into the next frame's start time.
    fn drop(&mut self) {
        // One borrow releases the witness and closes the cycle whether the frame was submitted
        // or not.
        task_context::with_animation_state(|state| {
            if !self.submitted {
                state.flush_and_end_cycle();
                error!(
                    "Frame was dropped without being submitted: {}:{}:{}",
                    self.created_at.file(),
                    self.created_at.line(),
                    self.created_at.column(),
                );
            }
            state.witness = None;
        });
    }
}
