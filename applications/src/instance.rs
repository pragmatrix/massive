use tokio::sync::mpsc::UnboundedSender;

use crate::{
    InstanceId, application_context::ApplicationRequest, view_builder::ViewBuilder,
};

#[derive(Debug)]
pub struct Instance {
    id: InstanceId,
    application: UnboundedSender<ApplicationRequest>,
}

impl Instance {
    pub fn new_view(&self, size: (u32, u32)) -> ViewBuilder {
        ViewBuilder::new(self.application.clone(), self.id, size)
    }
}
