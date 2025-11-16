use anyhow::Result;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{
    InstanceId,
    application_context::ApplicationRequest,
    view::{View, ViewClient, ViewRole},
};
use massive_geometry::Color;

#[derive(Debug)]
pub struct ViewBuilder {
    shell: UnboundedSender<ApplicationRequest>,
    instance: InstanceId,

    role: ViewRole,
    size: (u32, u32),

    background_color: Option<Color>,
}

impl ViewBuilder {
    pub(crate) fn new(
        application: UnboundedSender<ApplicationRequest>,
        instance: InstanceId,
        size: (u32, u32),
    ) -> Self {
        Self {
            shell: application,
            instance,
            size,
            role: ViewRole::default(),
            background_color: None,
        }
    }

    pub fn with_role(mut self, role: ViewRole) -> Self {
        self.role = role;
        self
    }

    pub fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn build(self) -> Result<View> {
        let (event_tx, event_rx) = unbounded_channel();
        let client = ViewClient::new(self.instance, self.role, event_tx);
        self.shell.send(ApplicationRequest::Present(client))?;
        Ok(View::new(self.shell, event_rx))
    }
}
