use derive_more::From;
use uuid::Uuid;

mod application_context;
mod instance;
mod instance_context;
mod view;
mod view_builder;

pub use application_context::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
struct InstanceId(Uuid);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct ApplicationId(Uuid);
