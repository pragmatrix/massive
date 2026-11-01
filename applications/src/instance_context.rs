//! The context for an instance.

use std::any::Any;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use log::{error, trace, warn};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedReceiver;

use massive_renderer::RenderPacing;
use massive_scene::{Location, Ref, SceneChange};
use massive_util::{ChangeCollector, ChangeSet, CoalescingReceiver};

use crate::prelude::*;
use crate::view_builder::ViewBuilder;
use crate::{
    ApplicationEvent, ApplicationMessage, ConfigurationRequest, FrameSubmission, InstanceChange,
    InstanceEnvironment, InstanceId, InstanceOutput, InstanceParameters, InstanceSubmission,
    ToolCallId, ToolError, ViewExtent, encode_result,
};
use crate::{pacing_for, task_context};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationMode {
    New(InstanceParameters),
    Restore,
}

/// The instance's change queue: scene changes interleaved with instance changes (ADR 0008).
pub type InstanceChangeCollector = ChangeCollector<InstanceChange>;

impl From<SceneChange> for InstanceChange {
    fn from(change: SceneChange) -> Self {
        Self::Scene(change)
    }
}

#[derive(Debug)]
pub struct InstanceContext {
    id: InstanceId,
    creation_mode: CreationMode,
    environment: InstanceEnvironment,
    view_parent: Ref<Location>,

    /// This is here so that we don't submit empty instance submissions when the pacing did not
    /// change.
    last_submitted_pacing: RenderPacing,

    events: CoalescingReceiver<ApplicationMessage>,

    /// The shell's window factory, present when the instance draws its own windows
    /// ([`Presentation::Windows`](crate::Presentation::Windows)).
    host_windows: Option<HostWindows>,
}

/// The window factory of the host, kept opaque here because the shell that defines it depends on
/// this crate. The shell's `InstanceWindows` trait reads it back.
pub type HostWindows = Arc<dyn Any + Send + Sync>;

impl Drop for InstanceContext {
    fn drop(&mut self) {
        // An instance that draws its own windows collects scene changes and renders them itself,
        // so it has no instance changes to submit.
        if self.host_windows.is_some() {
            return;
        }
        warn!("Submitting final instance changes: instance={:?}", self.id);
        // If the instance ends, we _must_ submit all pending changes. The End is pushed last so
        // the desktop observes it behind every pending change of this submission.
        submit(InstanceChange::End(self.view_parent.clone()));
        // Teardown runs after the run loop returned, so no frame is live. The detached frame end
        // witnesses animation access for this one teardown (joining, not replacing, a frame held
        // across a panic unwind), flushes queued movement actions, closes the cycle and never
        // panics — this is a Drop. How the cycle ends is the final pacing.
        let pacing = pacing_for(task_context::end_frame_cycle_detached());
        if let Err(e) = self.submit_with_pacing(pacing) {
            error!("Final instance submit error for {:?}: {e:?}", self.id);
        }
    }
}

impl InstanceContext {
    pub fn new(
        id: InstanceId,
        creation_mode: CreationMode,
        environment: InstanceEnvironment,
        view_parent: Ref<Location>,
        events: UnboundedReceiver<ApplicationMessage>,
    ) -> Self {
        Self {
            id,
            creation_mode,
            environment,
            view_parent,
            last_submitted_pacing: RenderPacing::Fast,
            events: events.into(),
            host_windows: None,
        }
    }

    /// Lets the instance draw its own windows through `windows`. Only the shell does this.
    pub fn with_host_windows(mut self, windows: HostWindows) -> Self {
        self.host_windows = Some(windows);
        self
    }

    pub fn host_windows(&self) -> Option<&HostWindows> {
        self.host_windows.as_ref()
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    pub fn creation_mode(&self) -> &CreationMode {
        &self.creation_mode
    }

    pub fn parameters(&self) -> Option<&InstanceParameters> {
        match &self.creation_mode {
            CreationMode::New(map) => Some(map),
            CreationMode::Restore => None,
        }
    }

    /// The environment variables an instance sets for every process it starts, so that a tool
    /// client started there connects to this desktop and names this instance as its target
    /// (ADR 0036).
    pub fn process_environment(&self) -> [(&'static str, String); 2] {
        [
            (
                "MASSIVE_SOCKET",
                self.environment.desktop_socket.display().to_string(),
            ),
            // The instance id as a hyphenated UUID, which the desktop parses back.
            ("MASSIVE_INSTANCE", self.id.to_string()),
        ]
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.environment.primary_monitor_scale_factor
    }

    pub async fn wait_for_event(&mut self) -> Result<ApplicationEvent<std::convert::Infallible>> {
        Ok(self.events.recv().await?.into())
    }

    /// Waits for the next events, all that are pending, and treats custom events of type `T`.
    ///
    /// On an animation tick, the task's animation cycle starts at the tick's vblank time and the
    /// movements advance to it, before the instance opens its next frame. A movement's completion
    /// event arrives as [`ApplicationEvent::Custom`], and one that is not a `T` is an error.
    pub async fn wait_for_events<T: Any>(&mut self) -> Result<Vec<ApplicationEvent<T>>> {
        let messages = self.events.recv_all().await?;

        let mut events = Vec::with_capacity(messages.len());
        for message in messages {
            let ApplicationMessage::ApplyAnimations(tick) = message else {
                events.push(message.into());
                continue;
            };
            let completion_events =
                task_context::with_animation_and_movement(|animation, movement| {
                    animation.upgrade_to_apply_animations_cycle(tick.vblank_time);
                    movement.apply_animations(animation.animation_time())
                });
            events.push(ApplicationEvent::ApplyAnimations(tick));
            for event in completion_events {
                let event = event
                    .downcast::<T>()
                    .map_err(|_| anyhow!("movement completion event has the wrong type"))?;
                events.push(ApplicationEvent::Custom(*event));
            }
        }

        Ok(events)
    }

    pub fn view(&self, extent: impl Into<ViewExtent>) -> ViewBuilder {
        assert!(
            self.host_windows.is_none(),
            "an instance that draws its own windows has no views"
        );
        ViewBuilder::new(self.view_parent.clone(), extent.into().into())
    }

    /// Design: This may interfere with animations and requires a final submit()!
    pub fn collect_configuration_request(&mut self, request: ConfigurationRequest) {
        submit(InstanceChange::Configuration(request));
    }

    /// Answers a tool call.
    ///
    /// The answer is sent at once, apart from the collected changes, so a waiting agent does not
    /// depend on the next frame. The answer stays ordered behind everything sent with
    /// [`Self::submit`] before, so a tool whose answer reports view changes submits them first. A
    /// call may be answered long after it arrived (ADR 0036).
    pub fn answer_tool_call(
        &self,
        call_id: ToolCallId,
        result: Result<impl Serialize, ToolError>,
    ) -> Result<()> {
        self.send(InstanceOutput::ToolResult {
            call_id,
            outcome: result.map(|output| encode_result(&output)),
        })
    }

    pub fn submit(&mut self, submission: FrameSubmission<InstanceChange>) -> Result<()> {
        let (changes, pacing) = submission.into_parts();
        self.submit_changes(changes, pacing)
    }

    fn submit_with_pacing(&mut self, pacing: RenderPacing) -> Result<()> {
        let changes = task_context::take_changes::<InstanceChange>();
        self.submit_changes(changes, pacing)
    }

    fn submit_changes(
        &mut self,
        changes: ChangeSet<InstanceChange>,
        pacing: RenderPacing,
    ) -> Result<()> {
        assert!(
            self.host_windows.is_none(),
            "an instance that draws its own windows renders its changes itself"
        );
        let change_count = changes.len();
        // Desktop needs empty submissions to observe pacing transitions, but repeated pacing has
        // no effect.
        if change_count == 0 && pacing == self.last_submitted_pacing {
            return Ok(());
        }

        trace!(
            "Submitting instance changes: instance={:?}, changes={change_count}, pacing={pacing:?}",
            self.id
        );

        self.send(InstanceOutput::Submission(InstanceSubmission::new(
            changes, pacing,
        )))?;
        self.last_submitted_pacing = pacing;
        Ok(())
    }

    fn send(&self, output: InstanceOutput) -> Result<()> {
        if let Err(e) = self.environment.output_sender.send((self.id, output)) {
            bail!(
                "Failed to send instance output because the desktop's receiver is closed: instance={:?}, err: {e:?}",
                self.id
            );
        }
        Ok(())
    }
}
