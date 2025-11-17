use derive_more::From;
use uuid::Uuid;

mod instance_client;
mod instance_context;
mod view;
mod view_builder;

pub use instance_context::*;
pub use view::{View, ViewClient, ViewRole};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct InstanceId(Uuid);
