//! The context for a module.

use anyhow::Result;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    instance::Instance,
    view::{ViewClient, ViewRequest},
};

#[derive(Debug)]
struct ApplicationContext {
    events: UnboundedReceiver<ApplicationEvent>,
    requests: UnboundedSender<ApplicationRequest>,
}

impl ApplicationContext {
    pub fn wait_for_event() -> Result<ApplicationEvent> {
        todo!();
    }
}

#[derive(Debug)]
enum ApplicationEvent {
    Materialize(Instance),
    Exit,
}

pub enum ApplicationRequest {
    Present(ViewClient),
    View(ViewRequest),
}
