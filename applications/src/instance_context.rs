//! The context for an instance.

use anyhow::Result;
use anyhow::anyhow;
use massive_animation::AnimationCoordinator;
use massive_renderer::FontManager;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    InstanceId, Scene, ViewEvent, ViewId, view::ViewCommand, view::ViewCreationInfo,
    view_builder::ViewBuilder,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CreationMode {
    New,
    Restore,
}

#[derive(Debug)]
pub struct InstanceContext {
    id: InstanceId,
    creation_mode: CreationMode,
    primary_monitor_scale_factor: f64,
    fonts: FontManager,

    /// The AnimationCoordinator is here to create new scenes. There is one per instance for now.
    animation_coordinator: AnimationCoordinator,
    events: UnboundedReceiver<InstanceEvent>,
    command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
}

impl InstanceContext {
    pub fn new(
        id: InstanceId,
        creation_mode: CreationMode,
        // Robustness: This might change on runtime.
        primary_monitor_scale_factor: f64,
        fonts: FontManager,
        requests: UnboundedSender<(InstanceId, InstanceCommand)>,
        events: UnboundedReceiver<InstanceEvent>,
    ) -> Self {
        // ADR: Every instance gets its own animation coordinator and its timestamp is reset as soon
        // the scene is rendered. This way, consistence can be preserved when animations are applied
        // in several instances in parallel. Otherwise timestamps from one instance could affect the
        // other.
        Self {
            id,
            creation_mode,
            primary_monitor_scale_factor,
            fonts,
            animation_coordinator: AnimationCoordinator::new(),
            events,
            command_sender: requests,
        }
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    pub fn creation_mode(&self) -> CreationMode {
        self.creation_mode
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.primary_monitor_scale_factor
    }

    pub fn fonts(&self) -> &FontManager {
        &self.fonts
    }

    pub fn new_scene(&self) -> Scene {
        Scene::new(self.animation_coordinator.clone())
    }

    pub async fn wait_for_event(&mut self) -> Result<InstanceEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow!("Instance event channel closed"))
            .map(|e| {
                if matches!(e, InstanceEvent::ApplyAnimations) {
                    self.animation_coordinator.upgrade_to_apply_animations();
                }
                e
            })
    }

    pub fn view(&self, size: (u32, u32)) -> ViewBuilder {
        ViewBuilder::new(self.command_sender.clone(), self.id, size)
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
    DestroyView(ViewId),
    View(ViewId, ViewCommand),
}
