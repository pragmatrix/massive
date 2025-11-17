use tokio::sync::mpsc::UnboundedSender;

use crate::{InstanceId, instance_context::InstanceRequest, view_builder::ViewBuilder};

#[derive(Debug)]
pub struct Instance {
    id: InstanceId,
    requests: UnboundedSender<InstanceRequest>,
}

impl Instance {
    pub(crate) fn new(id: InstanceId, requests: UnboundedSender<InstanceRequest>) -> Self {
        Self { id, requests }
    }

    pub fn new_view(&self, size: (u32, u32)) -> ViewBuilder {
        ViewBuilder::new(self.requests.clone(), self.id, size)
    }
}
