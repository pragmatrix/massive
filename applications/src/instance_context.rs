//! The context for an instance.

use anyhow::Result;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    InstanceId,
    view::{ViewClient, ViewRequest},
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
    events: UnboundedReceiver<InstanceEvent>,
    requests: UnboundedSender<(InstanceId, InstanceRequest)>,
}

impl InstanceContext {
    pub fn new(
        id: InstanceId,
        creation_mode: CreationMode,
        requests: UnboundedSender<(InstanceId, InstanceRequest)>,
        events: UnboundedReceiver<InstanceEvent>,
    ) -> Self {
        Self {
            id,
            creation_mode,
            events,
            requests,
        }
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    pub fn creation_mode(&self) -> CreationMode {
        self.creation_mode
    }

    pub async fn wait_for_event(&mut self) -> Result<InstanceEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Instance event channel closed"))
    }

    fn send_request(&self, request: InstanceRequest) -> Result<()> {
        self.requests
            .send((self.id, request))
            .map_err(|_| anyhow::anyhow!("Request channel closed"))
    }
}

#[derive(Debug)]
pub enum InstanceEvent {
    Exit,
}

#[derive(Debug)]
pub enum InstanceRequest {
    Present(ViewClient),
    View(ViewRequest),
}
