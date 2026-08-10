//! The context for an instance.

use std::sync::Arc;

use anyhow::{Result, bail};
use derive_more::Deref;
use log::{error, trace, warn};
use tokio::sync::mpsc::UnboundedReceiver;

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_renderer::{FontManager, RenderPacing};
use massive_scene::{HandleChangeReceiver, Location, Ref, SceneChange};
use massive_util::CoalescingReceiver;

use crate::view_builder::ViewBuilder;
use crate::{
    ApplicationEvent, ApplicationMessage, ConfigurationRequest, Frame, FrameSubmission,
    InstanceChange, InstanceEnvironment, InstanceId, InstanceParameters, InstanceSubmission, Scene,
    ViewExtent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationMode {
    New(InstanceParameters),
    Restore,
}

// Need a newtype here for the orphan rule.
#[derive(Debug, Default, Deref)]
pub struct InstanceChangeCollector(massive_util::ChangeCollector<InstanceChange>);

impl HandleChangeReceiver for InstanceChangeCollector {
    fn send(&self, change: SceneChange) {
        self.0.collect(InstanceChange::Scene(change))
    }
}

#[derive(Debug)]
pub struct InstanceContext {
    id: InstanceId,
    creation_mode: CreationMode,
    environment: InstanceEnvironment,
    view_parent: Ref<Location>,

    /// We currently use one Scene per Context, so that everything is ordered properly. This also
    /// contains the AnimationCoordinator, which we need one only per instance anyway.
    animation_coordinator: AnimationCoordinator,
    movement_runtime: MovementRuntime,

    /// The current changes of this instance. This includes all Scene changes interleaved with the
    /// instance changes (in order).
    changes: Arc<InstanceChangeCollector>,

    /// This is here so that we don't submit empty instance submissions when the pacing did not
    /// change.
    last_submitted_pacing: RenderPacing,

    events: CoalescingReceiver<ApplicationMessage>,
}

impl Drop for InstanceContext {
    fn drop(&mut self) {
        warn!("Submitting final instance changes: instance={:?}", self.id);
        // If the instance ends, we _must_ submit all pending changes.
        self.changes
            .collect(InstanceChange::End(self.view_parent.clone()));
        let pacing = if self.animation_coordinator.end_cycle() {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };
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
        // ADR: Every instance gets its own animation coordinator and its timestamp is reset as soon
        // the scene is rendered. This way, consistence can be preserved when animations are applied
        // in several instances in parallel. Otherwise, timestamps from one instance could affect the
        // other.
        let animation_coordinator = AnimationCoordinator::new();

        // ADR: Every instance gets its own change collector, because of ordering constraints
        // between the commands sent to the desktop and the scene updates (they must be processed in
        // order by the desktop, otherwise it could happen that Visual refer to Locations /
        // Transforms that are not available anymore).
        let changes = InstanceChangeCollector::default();

        Self {
            id,
            creation_mode,
            environment,
            view_parent,
            animation_coordinator,
            movement_runtime: MovementRuntime::default(),
            changes: changes.into(),
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

    pub fn fonts(&self) -> &FontManager {
        &self.environment.font_manager
    }

    /// ADR: We share _one_ single scene in all views now, so that we can keep the updates that we
    /// send to desktop coordinated. Also, changes can't be submitted independently, all updates
    /// from all views need to be submitted at once.
    pub fn new_scene(&self) -> Scene {
        Scene::new(self.changes.clone())
    }

    /// Bundle a scene with this instance's animation clock for one update cycle.
    pub fn frame<'scene, 'context>(
        &'context mut self,
        scene: &'scene Scene,
    ) -> Frame<'scene, 'context> {
        Frame::new(
            scene,
            &mut self.animation_coordinator,
            &mut self.movement_runtime,
        )
    }

    pub async fn wait_for_event(&mut self) -> Result<ApplicationEvent<std::convert::Infallible>> {
        Ok(self.events.recv().await?.into())
    }

    pub fn view(&self, extent: impl Into<ViewExtent>) -> ViewBuilder {
        ViewBuilder::new(
            self.changes.clone(),
            self.view_parent.clone(),
            extent.into().into(),
            self.new_scene(),
        )
    }

    /// Design: This may interfere with animations and requires a final submit()!
    pub fn collect_desktop_request(&mut self, request: ConfigurationRequest) {
        self.changes.collect(InstanceChange::Configuration(request))
    }

    pub fn submit(&mut self, submission: FrameSubmission<'_>) -> Result<()> {
        self.submit_with_pacing(submission.pacing())
    }

    fn submit_with_pacing(&mut self, pacing: RenderPacing) -> Result<()> {
        let changes = self.changes.take_all();
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
