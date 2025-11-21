//! The context for an instance.

use anyhow::Result;
use anyhow::anyhow;
use massive_animation::AnimationCoordinator;
use massive_scene::{Handle, Location};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    InstanceId, Scene, ViewEvent, ViewId, ViewRole, view::ViewCommand, view_builder::ViewBuilder,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CreationMode {
    New,
    Restore,
}

#[derive(Debug)]
pub struct InstanceContext {
    /// ... to create new scenes.
    animation_coordinator: AnimationCoordinator,
    id: InstanceId,
    creation_mode: CreationMode,
    events: UnboundedReceiver<InstanceEvent>,
    command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
}

impl InstanceContext {
    pub fn new(
        animation_coordinator: AnimationCoordinator,
        id: InstanceId,
        creation_mode: CreationMode,
        requests: UnboundedSender<(InstanceId, InstanceCommand)>,
        events: UnboundedReceiver<InstanceEvent>,
    ) -> Self {
        Self {
            animation_coordinator,
            id,
            creation_mode,
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

    pub fn new_scene(&self) -> Scene {
        Scene::new(self.animation_coordinator.clone())
    }

    pub async fn wait_for_event(&mut self) -> Result<InstanceEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow!("Instance event channel closed"))
    }

    pub fn view(&self, size: (u32, u32)) -> ViewBuilder {
        ViewBuilder::new(self.command_sender.clone(), self.id, size)
    }

    fn send_request(&self, request: InstanceCommand) -> Result<()> {
        self.command_sender
            .send((self.id, request))
            .map_err(|_| anyhow!("Command channel closed"))
    }
}

#[derive(Debug)]
pub enum InstanceEvent {
    View(ViewId, ViewEvent),
    /// Destroy the whole instance.
    Shutdown,
}

#[derive(Debug)]
pub enum InstanceCommand {
    CreateView {
        id: ViewId,
        location: Handle<Location>,
        role: ViewRole,
        size: (u32, u32),
    },
    DestroyView(ViewId),
    View(ViewId, ViewCommand),
}
