use anyhow::{Result, anyhow};
use derive_more::Constructor;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use massive_scene::SceneChange;

use crate::{PersistenceId, application_context::ApplicationRequest, persistence};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
/// Some ideas for roles.
pub enum PresenceRole {
    #[default]
    Primary,
    Assistant,
    PersistentNotification,
    TemporaryNotification,
}

#[derive(Debug)]
pub struct Presence {
    shell: UnboundedSender<ApplicationRequest>,
    events: UnboundedReceiver<PresenceEvent>,
}

impl Presence {
    pub(crate) fn new(
        application: UnboundedSender<ApplicationRequest>,
        receiver: UnboundedReceiver<PresenceEvent>,
    ) -> Self {
        Self {
            shell: application,
            events: receiver,
        }
    }

    pub async fn wait_for_event(&mut self) -> Result<PresenceEvent> {
        self.events.recv().await.ok_or(anyhow!(
            "Internal error: Presence client vanished unexpectedly"
        ))
    }
}

#[derive(Debug)]
pub enum PresenceEvent {}

#[derive(Debug)]
pub enum PresenceRequest {
    /// Detail: Empty changes should not be possible. It should create an error. Compared to a
    /// window environment, there is no redraw needed when there are no changes.
    Redraw(Vec<SceneChange>),
    /// Feature: This should probably specify a depth too.
    Resize((u32, u32)),
    /// Can't we do this automatically through the Scene?
    ChangePacing(),
}

/// The side of a presence the shell sees.
#[derive(Debug)]
pub struct PresenceClient {
    persistence: PersistenceId,
    role: PresenceRole,
    events: UnboundedSender<PresenceEvent>,
}

impl PresenceClient {
    pub(crate) fn new(
        persistence: PersistenceId,
        role: PresenceRole,
        events: UnboundedSender<PresenceEvent>,
    ) -> Self {
        Self {
            persistence,
            role,
            events,
        }
    }
}
