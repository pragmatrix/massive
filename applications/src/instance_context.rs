use std::collections::HashSet;

use crate::{InstanceId, view::ViewRole};
use anyhow::Result;
use uuid::Uuid;

#[derive(Debug)]
struct InstanceClient {
    supported_roles: HashSet<ViewRole>,
}

impl InstanceClient {
    pub fn wait_for_event() -> Result<InstanceEvent> {
        todo!()
    }
}

#[derive(Debug)]
struct ViewId(Uuid);

enum InstanceEvent {
    Presented(ViewId, ViewRole, (u32, u32)),
    Withdrawn(ViewId),
    Resized(ViewId, (u32, u32)),
}
