//! The context for a module.

use anyhow::Result;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    ApplicationId,
    instance::Instance,
    view::{ViewClient, ViewRequest},
};

#[derive(Debug)]
pub struct ApplicationContext {
    id: ApplicationId,
    events: UnboundedReceiver<ApplicationEvent>,
    requests: UnboundedSender<(ApplicationId, ApplicationRequest)>,
}

impl ApplicationContext {
    pub fn new(
        id: ApplicationId,
        requests: UnboundedSender<(ApplicationId, ApplicationRequest)>,
        events: UnboundedReceiver<ApplicationEvent>,
    ) -> Self {
        Self {
            id,
            events,
            requests,
        }
    }

    pub async fn wait_for_event(&mut self) -> Result<ApplicationEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Application event channel closed"))
    }

    fn send_request(&self, request: ApplicationRequest) -> Result<()> {
        self.requests
            .send((self.id, request))
            .map_err(|_| anyhow::anyhow!("Request channel closed"))
    }
}

#[derive(Debug)]
pub enum ApplicationEvent {
    Materialize(Instance),
    Exit,
}

#[derive(Debug)]
pub enum ApplicationRequest {
    Present(ViewClient),
    View(ViewRequest),
}
