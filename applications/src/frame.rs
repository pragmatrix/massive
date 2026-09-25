//! A short-lived bundle of one animation cycle over the task's change queue.
//!
//! ADR: The [`AnimationCoordinator`] is owned by exactly one context (an instance or the
//! application), while the change queue is installed per task. The frame opens and closes the
//! animation cycle; the task's queue is drained at submission time, so its ownership stays
//! with the task context (ADR 0008).
//!
//! The frame itself is change-kind agnostic: the kind is fixed where the frame is consumed.
//! A render submission exists only for the application task's scene queue
//! ([`Frame::render_to`], [`Frame::render_submission`]), while [`Frame::submission`] is
//! generic and takes its kind from the submission call — an instance's
//! `InstanceContext::submit` fixes `InstanceChange`. The typed task-local accessor downcasts
//! and panics loudly on a kind mismatch, so draining the wrong queue names the wrong type
//! instead of mixing submissions silently.

use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;

use log::error;

use massive_renderer::{RenderPacing, RenderSubmission};
use massive_scene::SceneChange;
use massive_util::ChangeSet;

use crate::task_context;

/// The bounds a frame's change kind `C` must satisfy: it converts from [`SceneChange`] and is
/// the collected type of the task's change queue (see [`task_context::take_changes`]).
///
/// Stated once here instead of on every submission item. It is public so generic code over
/// [`FrameSubmission`] can name it; the blanket impl covers every type that satisfies the
/// bounds.
pub trait Change: From<SceneChange> + fmt::Debug + Send + 'static {}

impl<C> Change for C where C: From<SceneChange> + fmt::Debug + Send + 'static {}

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
/// The change kind follows from how the frame is consumed, so one opener serves both task
/// kinds: [`Frame::render_to`] and [`Frame::render_submission`] drain the application task's
/// [`SceneChange`] queue, while [`Frame::submission`] takes the kind of the submission call
/// that receives it. A task context must be installed: the frame reads the task's animation
/// clock.
#[track_caller]
pub fn begin_frame() -> Frame {
    let created_at = Location::caller();

    // One call installs the witness and begins the cycle. Failing here means a frame is still
    // live; the returned site names the blocking frame so both frames appear in the panic.
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
        task_context::with_animation(|animation| animation.upgrade_to_apply_animations_cycle());
    }

    /// The application task's render submission: its queue drained as [`SceneChange`]s.
    pub fn render_submission(self) -> RenderSubmission {
        self.submission::<SceneChange>().into_render_submission()
    }

    /// End the animation cycle and drain the task's change queue into a submission of the
    /// installed change kind `C`.
    pub fn submission<C: Change>(mut self) -> FrameSubmission<C> {
        let pacing = self.end_cycle();

        FrameSubmission {
            changes: task_context::take_changes::<C>(),
            pacing,
        }
    }

    fn end_cycle(&mut self) -> RenderPacing {
        self.submitted = true;

        // Flushes queued movement actions before closing the cycle.
        if task_context::end_frame_cycle() {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        }
    }
}

impl Drop for Frame {
    // Release-only, unconditionally: frames are legitimately dropped unsubmitted on the normal
    // quit paths (e.g. the desktop's CloseRequested return, which then opens further frames
    // during shutdown), so a missing submit must never poison the witness for the next frame.
    // The cycle is closed here too, so a dropped frame cannot leak its open cycle into the next
    // frame's cycle start time.
    fn drop(&mut self) {
        if !self.submitted {
            // Terminate the cycle and flush queued movement actions the same way a submission
            // would.
            task_context::end_frame_cycle();

            error!(
                "Frame was dropped without being submitted: {}:{}:{}",
                self.created_at.file(),
                self.created_at.line(),
                self.created_at.column(),
            );
        }

        task_context::release_frame_witness();
    }
}
