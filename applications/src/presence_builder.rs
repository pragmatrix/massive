use anyhow::Result;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{
    PersistenceId,
    application_context::ApplicationRequest,
    presence::{Presence, PresenceClient, PresenceRole},
};
use massive_geometry::Color;

#[derive(Debug)]
pub struct PresenceBuilder {
    shell: UnboundedSender<ApplicationRequest>,
    persistence: PersistenceId,

    role: PresenceRole,
    size: (u32, u32),

    background_color: Option<Color>,
}

impl PresenceBuilder {
    pub(crate) fn new(
        application: UnboundedSender<ApplicationRequest>,
        persistence: PersistenceId,
        size: (u32, u32),
    ) -> Self {
        Self {
            shell: application,
            persistence,
            size,
            role: PresenceRole::default(),
            background_color: None,
        }
    }

    pub fn with_role(mut self, role: PresenceRole) -> Self {
        self.role = role;
        self
    }

    pub fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn build(self) -> Result<Presence> {
        let (event_tx, event_rx) = unbounded_channel();
        let client = PresenceClient::new(self.persistence, self.role, event_tx);
        self.shell.send(ApplicationRequest::Present(client))?;
        Ok(Presence::new(self.shell, event_rx))
    }
}
