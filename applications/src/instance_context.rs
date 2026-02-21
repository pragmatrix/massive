//! The context for an instance.

use std::{mem, sync::Arc};

use anyhow::Result;
use massive_scene::ChangeCollector;
use tokio::sync::mpsc::UnboundedReceiver;

use massive_animation::AnimationCoordinator;
use massive_renderer::FontManager;
use massive_util::{CoalescingKey, CoalescingReceiver};

use crate::{
    InstanceEnvironment, InstanceId, InstanceParameters, Scene, ViewEvent, ViewExtent, ViewId,
    view::{ViewCommand, ViewCreationInfo},
    view_builder::ViewBuilder,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationMode {
    New(InstanceParameters),
    Restore,
}

#[derive(Debug)]
pub struct InstanceContext {
    id: InstanceId,
    creation_mode: CreationMode,
    environment: InstanceEnvironment,

    /// The AnimationCoordinator is here to create new scenes. There is one per instance for now.
    animation_coordinator: AnimationCoordinator,
    events: CoalescingReceiver<InstanceEvent>,
}

impl InstanceContext {
    pub fn new(
        id: InstanceId,
        creation_mode: CreationMode,
        environment: InstanceEnvironment,
        events: UnboundedReceiver<InstanceEvent>,
    ) -> Self {
        // ADR: Every instance gets its own animation coordinator and its timestamp is reset as soon
        // the scene is rendered. This way, consistence can be preserved when animations are applied
        // in several instances in parallel. Otherwise timestamps from one instance could affect the
        // other.
        Self {
            id,
            creation_mode,
            environment,
            animation_coordinator: AnimationCoordinator::new(),
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

    pub fn new_scene(&self) -> Scene {
        Scene::new(self.animation_coordinator.clone())
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
            self.environment.command_sender.clone(),
            self.id,
            extent.into().into(),
            self.new_scene(),
        )
    }
}

#[derive(Debug, Clone)]
pub enum InstanceEvent {
    View(ViewId, ViewEvent),
    /// Destroy the whole instance.
    Shutdown,
    ApplyAnimations,
}

#[derive(Debug)]
pub enum InstanceCommand {
    CreateView(ViewCreationInfo),
    // Detail: We pass the change collector up to the desktop, so it can make all Handles are destroyed and
    // pending changes are sent to the renderer.
    DestroyView(ViewId, Arc<ChangeCollector>),
    View(ViewId, ViewCommand),
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
                ViewEvent::CursorMoved { .. } => Some(InstanceEventCoalescingKey::ViewEvent(
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
