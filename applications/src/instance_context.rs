//! The context for an instance.

use anyhow::{Result, bail};
use log::{error, trace, warn};
use tokio::sync::mpsc::UnboundedReceiver;

use massive_renderer::RenderPacing;
use massive_scene::{Location, Ref, SceneChange};
use massive_util::{ChangeCollector, ChangeSet, CoalescingReceiver};

use crate::prelude::*;
use crate::view_builder::ViewBuilder;
use crate::{
    ApplicationEvent, ApplicationMessage, ConfigurationRequest, FrameSubmission, InstanceChange,
    InstanceEnvironment, InstanceId, InstanceParameters, InstanceSubmission, ViewExtent,
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
}

impl Drop for InstanceContext {
    fn drop(&mut self) {
        warn!("Submitting final instance changes: instance={:?}", self.id);
        // If the instance ends, we _must_ submit all pending changes. The End is pushed last so
        // the desktop observes it behind every pending change of this submission.
        collect(InstanceChange::End(self.view_parent.clone()));
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

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.environment.primary_monitor_scale_factor
    }

    pub async fn wait_for_event(&mut self) -> Result<ApplicationEvent<std::convert::Infallible>> {
        Ok(self.events.recv().await?.into())
    }

    pub fn view(&self, extent: impl Into<ViewExtent>) -> ViewBuilder {
        ViewBuilder::new(self.view_parent.clone(), extent.into().into())
    }

    /// Design: This may interfere with animations and requires a final submit()!
    pub fn collect_configuration_request(&mut self, request: ConfigurationRequest) {
        collect(InstanceChange::Configuration(request));
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

        let submission = InstanceSubmission::new(changes, pacing);
        if let Err(e) = self
            .environment
            .submission_sender
            .send((self.id, submission))
        {
            bail!(
                "Failed to submit instance changes because the desktop submission receiver is closed: instance={:?}, changes={change_count}, err: {e:?}",
                self.id
            );
        }

        self.last_submitted_pacing = pacing;

        Ok(())
    }
}
