use std::collections::HashSet;

use crate::{PersistenceId, presence::PresenceRole};
use anyhow::Result;
use uuid::Uuid;

#[derive(Debug)]
struct PersistenceClient {
    supported_roles: HashSet<PresenceRole>,
}

impl PersistenceClient {
    pub fn wait_for_event() -> Result<PersistenceEvent> {
        todo!()
    }
}

#[derive(Debug)]
struct PresenceId(Uuid);

enum PersistenceEvent {
    Presented(PresenceId, PresenceRole, (u32, u32)),
    Withdrawn(PresenceId),
    Resized(PresenceId, (u32, u32)),
}
