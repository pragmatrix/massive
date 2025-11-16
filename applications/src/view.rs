use anyhow::{Result, anyhow};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use massive_scene::SceneChange;

use crate::{InstanceId, application_context::ApplicationRequest, instance};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
/// Some ideas for roles.
pub enum ViewRole {
    #[default]
    Primary,
    Assistant,
    Notification {
        persistent: bool,
    },
}

#[derive(Debug)]
pub struct View {
    shell: UnboundedSender<ApplicationRequest>,
    events: UnboundedReceiver<ViewEvent>,
}

impl View {
    pub(crate) fn new(
        application: UnboundedSender<ApplicationRequest>,
        receiver: UnboundedReceiver<ViewEvent>,
    ) -> Self {
        Self {
            shell: application,
            events: receiver,
        }
    }

    pub async fn wait_for_event(&mut self) -> Result<ViewEvent> {
        self.events
            .recv()
            .await
            .ok_or(anyhow!("Internal error: View client vanished unexpectedly"))
    }
}

#[derive(Debug)]
pub enum ViewEvent {}

#[derive(Debug)]
pub enum ViewRequest {
    /// Detail: Empty changes should not be possible. It should create an error. Compared to a
    /// window environment, there is no redraw needed when there are no changes.
    Redraw(Vec<SceneChange>),
    /// Feature: This should probably specify a depth too.
    Resize((u32, u32)),
    /// Can't we do this automatically through the Scene?
    ChangePacing(),
}

/// The side of a view the shell sees.
#[derive(Debug)]
pub struct ViewClient {
    instance: InstanceId,
    role: ViewRole,
    events: UnboundedSender<ViewEvent>,
}

impl ViewClient {
    pub(crate) fn new(
        instance: InstanceId,
        role: ViewRole,
        events: UnboundedSender<ViewEvent>,
    ) -> Self {
        Self {
            instance,
            role,
            events,
        }
    }
}
