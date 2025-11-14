use tokio::sync::mpsc::UnboundedSender;

use crate::{
    PersistenceId, application_context::ApplicationRequest, presence_builder::PresenceBuilder,
};

#[derive(Debug)]
pub struct Persistence {
    id: PersistenceId,
    application: UnboundedSender<ApplicationRequest>,
}

impl Persistence {
    pub fn new_presence(&self, size: (u32, u32)) -> PresenceBuilder {
        PresenceBuilder::new(self.application.clone(), self.id, size)
    }
}
