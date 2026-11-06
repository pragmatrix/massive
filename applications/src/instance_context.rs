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
    ConfigurationRequest, FrameSubmission, InstanceChange, InstanceEnvironment, InstanceEvent,
    InstanceId, InstanceMessage, InstanceOutput, InstanceParameters, InstanceSubmission,
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
    draw_target: DrawTargetState,
    events: CoalescingReceiver<InstanceMessage>,
}

/// How the instance's content reaches the screen, following the application's
/// [`DrawTarget`](crate::DrawTarget).
#[derive(Debug)]
enum DrawTargetState {
    /// The instance's views hang below `view_parent`, and the host draws the instance changes the
    /// instance submits.
    Views {
        view_parent: Ref<Location>,
        /// This is here so that we don't submit empty instance submissions when the pacing did
        /// not change.
        last_submitted_pacing: RenderPacing,
    },
    /// The instance opens its own windows through the host's window factory and renders its scene
    /// changes into them itself.
    Windows(HostWindows),
}

/// The window factory of the host, kept opaque here because the shell that defines it depends on
/// this crate. The shell's `InstanceWindows` trait reads it back.
pub type HostWindows = Arc<dyn Any + Send + Sync>;

impl Drop for InstanceContext {
    fn drop(&mut self) {
        let DrawTargetState::Views { view_parent, .. } = &self.draw_target else {
            return;
        };
        warn!("Submitting final instance changes: instance={:?}", self.id);
        // If the instance ends, we _must_ submit all pending changes. The End is pushed last so
        // the desktop observes it behind every pending change of this submission.
        submit(InstanceChange::End(view_parent.clone()));
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
    /// The context of an instance whose views hang below `view_parent` and whose host draws them.
    pub fn new(
        id: InstanceId,
        creation_mode: CreationMode,
        environment: InstanceEnvironment,
        view_parent: Ref<Location>,
        events: UnboundedReceiver<InstanceMessage>,
    ) -> Self {
        Self {
            id,
            creation_mode,
            environment,
            draw_target: DrawTargetState::Views {
                view_parent,
                last_submitted_pacing: RenderPacing::Fast,
            },
            events: events.into(),
        }
    }

    /// The context of an instance that opens its own windows through `windows`. Only the shell
    /// creates one.
    pub fn drawing_windows(
        id: InstanceId,
        creation_mode: CreationMode,
        environment: InstanceEnvironment,
        windows: HostWindows,
        events: UnboundedReceiver<InstanceMessage>,
    ) -> Self {
        Self {
            id,
            creation_mode,
            environment,
            draw_target: DrawTargetState::Windows(windows),
            events: events.into(),
        }
    }

    /// The host's window factory, if the instance draws its own windows.
    pub fn host_windows(&self) -> Option<&HostWindows> {
        match &self.draw_target {
            DrawTargetState::Views { .. } => None,
            DrawTargetState::Windows(windows) => Some(windows),
        }
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
    ///
    /// Under the shell, which has no desktop socket, there are none, so a tool client started
    /// there finds the desktop the way any other process does.
    pub fn process_environment(&self) -> Vec<(&'static str, String)> {
        let Some(desktop_socket) = &self.environment.desktop_socket else {
            return Vec::new();
        };
        vec![
            ("MASSIVE_SOCKET", desktop_socket.display().to_string()),
            // The instance id as a hyphenated UUID, which the desktop parses back.
            ("MASSIVE_INSTANCE", self.id.to_string()),
        ]
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.environment.primary_monitor_scale_factor
    }

    /// Waits for the next events, all that are pending, and treats custom events of type `T`.
    ///
    /// On an animation tick, the task's animation cycle starts at the tick's vblank time and the
    /// movements advance to it, before the instance opens its next frame. A movement's completion
    /// event arrives as [`InstanceEvent::Custom`], and one that is not a `T` is an error.
    pub async fn wait_for_events<T: Any>(&mut self) -> Result<Vec<InstanceEvent<T>>> {
        let messages = self.events.recv_all().await?;

        let mut events = Vec::with_capacity(messages.len());
        for message in messages {
            let InstanceMessage::ApplyAnimations(tick) = message else {
                events.push(message.into());
                continue;
            };
            let completion_events =
                task_context::with_animation_and_movement(|animation, movement| {
                    animation.upgrade_to_apply_animations_cycle(tick.vblank_time);
                    movement.apply_animations(animation.animation_time())
                });
            events.push(InstanceEvent::ApplyAnimations(tick));
            for event in completion_events {
                let event = event
                    .downcast::<T>()
                    .map_err(|_| anyhow!("movement completion event has the wrong type"))?;
                events.push(InstanceEvent::Custom(*event));
            }
        }

        Ok(events)
    }

    pub fn view(&self, extent: impl Into<ViewExtent>) -> ViewBuilder {
        let DrawTargetState::Views { view_parent, .. } = &self.draw_target else {
            panic!("an instance that draws its own windows has no views");
        };
        ViewBuilder::new(view_parent.clone(), extent.into().into())
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
        let DrawTargetState::Views {
            last_submitted_pacing,
            ..
        } = &mut self.draw_target
        else {
            panic!("an instance that draws its own windows renders its changes itself");
        };
        let change_count = changes.len();
        // Desktop needs empty submissions to observe pacing transitions, but repeated pacing has
        // no effect.
        if change_count == 0 && pacing == *last_submitted_pacing {
            return Ok(());
        }
        // A failed send ends the instance, so recording the pacing before it leaves no stale
        // record that is read later.
        *last_submitted_pacing = pacing;

        trace!(
            "Submitting instance changes: instance={:?}, changes={change_count}, pacing={pacing:?}",
            self.id
        );

        self.send(InstanceOutput::Submission(InstanceSubmission::new(
            changes, pacing,
        )))
    }

    fn send(&self, output: InstanceOutput) -> Result<()> {
        if let Err(e) = self.environment.output_sender.send((self.id, output)) {
            bail!(
                "Failed to send instance output because the host's receiver is closed: instance={:?}, err: {e:?}",
                self.id
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use massive_animation::{
        Animated, AnimationCoordinator, Interpolation, Movement, MovementRuntime,
    };
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::AnyCollector;
    use massive_scene::prelude::*;
    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

    use crate::task_context::{TaskContext, with_context};
    use crate::{PresentationId, PresentationTick};

    use super::*;

    const MOVEMENT_DURATION: Duration = Duration::from_millis(100);

    #[derive(Debug, PartialEq)]
    struct MovementCompleted;

    /// A tick past a movement's end advances the movement, and its completion event follows the
    /// tick in the same batch, so the instance sees the end in the frame that the tick opens.
    #[tokio::test]
    async fn a_tick_delivers_a_finished_movements_completion_after_the_tick() {
        with_context(instance_task_context(), async {
            let (mut context, events, _movement) = instance_with_a_moving_movement();
            events.send(tick_past_the_movement()).unwrap();

            let received = context
                .wait_for_events::<MovementCompleted>()
                .await
                .unwrap();

            assert!(
                matches!(
                    received.as_slice(),
                    [
                        InstanceEvent::ApplyAnimations(_),
                        InstanceEvent::Custom(MovementCompleted)
                    ]
                ),
                "{received:?}"
            );
        })
        .await
    }

    /// A completion event of another type than the one the instance waits for is an error, not a
    /// lost event.
    #[tokio::test]
    async fn a_completion_event_of_another_type_is_an_error() {
        with_context(instance_task_context(), async {
            let (mut context, events, _movement) = instance_with_a_moving_movement();
            events.send(tick_past_the_movement()).unwrap();

            let error = context.wait_for_events::<String>().await.unwrap_err();

            assert!(error.to_string().contains("wrong type"), "{error}");
        })
        .await
    }

    fn instance_task_context() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<InstanceChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            // Any engine compiled into this build works: the tests never shape.
            FontManager::bare(ShapingEngineKind::available()[0]).new_shaping_context(),
        )
    }

    /// An instance whose one movement animates for [`MOVEMENT_DURATION`] and then completes. The
    /// movement is returned, so it stays mounted while the test runs.
    fn instance_with_a_moving_movement() -> (
        InstanceContext,
        UnboundedSender<InstanceMessage>,
        Movement<Animated<f64>>,
    ) {
        let (events, events_receiver) = unbounded_channel();
        let (output_sender, _outputs) = unbounded_channel();
        let (_root_transform, root) = identity_location().submit();
        let mut context = InstanceContext::new(
            InstanceId::new(),
            CreationMode::Restore,
            InstanceEnvironment::new(output_sender, 1.0, None),
            root.to_ref(),
            events_receiver,
        );

        let frame = begin_frame();
        let movement = movement(Animated::from(0.0), |value, progress| {
            value.proceed_with(progress);
        })
        .completion_event(|| MovementCompleted)
        .mount();
        movement.modify(|value, allocator| {
            value.animate_with(allocator, 1.0, MOVEMENT_DURATION, Interpolation::Linear);
        });
        // Ending the frame's cycle starts the queued animation.
        context.submit(frame.submission()).unwrap();

        (context, events, movement)
    }

    fn tick_past_the_movement() -> InstanceMessage {
        InstanceMessage::ApplyAnimations(PresentationTick {
            presentation_id: PresentationId::new(),
            vblank_time: Instant::now() + MOVEMENT_DURATION * 2,
        })
    }
}
