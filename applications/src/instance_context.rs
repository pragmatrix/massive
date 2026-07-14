//! The context for an instance.

use std::mem;
use std::sync::Arc;

use anyhow::{Result, bail};
use derive_more::Deref;
use log::{error, trace, warn};
use tokio::sync::mpsc::UnboundedReceiver;

use massive_animation::AnimationCoordinator;
use massive_renderer::{FontManager, RenderPacing};
use massive_scene::{HandleChangeReceiver, Location, Ref, SceneChange};
use massive_util::{CoalescingKey, CoalescingReceiver};

use crate::view_builder::ViewBuilder;
use crate::{
    DesktopRequest, InstanceChange, InstanceEnvironment, InstanceId, InstanceParameters,
    InstanceSubmission, Scene, ViewEvent, ViewExtent, ViewId,
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

    /// The current changes of this instance. This includes all Scene changes interleaved with the
    /// instance changes (in order).
    changes: Arc<InstanceChangeCollector>,

    events: CoalescingReceiver<InstanceEvent>,
}

impl Drop for InstanceContext {
    fn drop(&mut self) {
        warn!("Submitting final instance changes: instance={:?}", self.id);
        // If the instance ends, we _must_ submit all pending changes.
        self.changes
            .collect(InstanceChange::End(self.view_parent.clone()));
        if let Err(e) = self.submit() {
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
        events: UnboundedReceiver<InstanceEvent>,
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
            changes: changes.into(),
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
        let scene = massive_scene::Scene::new(self.changes.clone());
        Scene::from_parts(scene, self.animation_coordinator.clone())
    }

    pub async fn wait_for_event(&mut self) -> Result<InstanceEvent> {
        let event = self.events.recv().await?;

        if matches!(event, InstanceEvent::ApplyAnimations) {
            self.animation_coordinator
                .upgrade_to_apply_animations_cycle();
        }

        Ok(event)
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
    pub fn collect_desktop_request(&mut self, request: DesktopRequest) {
        self.changes.collect(InstanceChange::Desktop(request))
    }

    pub fn submit(&mut self) -> Result<()> {
        // Robustness: To be really thread safe, we would need to collect the changes and end the
        // cycle in one go.
        let animations_active = self.animation_coordinator.end_cycle();

        // Empty changes need to end in a submission (we might have done some before, without ending
        // the animation cycle)

        let pacing = if animations_active {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };

        let changes = self.changes.take_all();
        let change_count = changes.len();
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

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum InstanceEvent {
    View(ViewId, ViewEvent),
    /// Destroy the whole instance.
    Shutdown,
    ApplyAnimations,
}

impl CoalescingKey for InstanceEvent {
    type Key = InstanceEventCoalescingKey;

    fn coalescing_key(&self) -> Option<InstanceEventCoalescingKey> {
        match self {
            InstanceEvent::View(view_id, view_event) => match view_event {
                ViewEvent::Resized(..) => Some(InstanceEventCoalescingKey::ViewEvent(
                    *view_id,
                    mem::discriminant(view_event),
                )),
                ViewEvent::CursorMoved(..) => Some(InstanceEventCoalescingKey::ViewEvent(
                    *view_id,
                    mem::discriminant(view_event),
                )),
                _ => None,
            },
            InstanceEvent::ApplyAnimations => Some(InstanceEventCoalescingKey::ApplyAnimations),
            InstanceEvent::Shutdown => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum InstanceEventCoalescingKey {
    ApplyAnimations,
    ViewEvent(ViewId, mem::Discriminant<ViewEvent>),
}
