use derive_more::From;
use uuid::Uuid;

mod instance_context;
mod instance;
mod instance_client;
mod view;
mod view_builder;

pub use instance_context::*;
pub use instance::Instance;
pub use view::{View, ViewClient, ViewRole};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct InstanceId(Uuid);
