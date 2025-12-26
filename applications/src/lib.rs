use derive_more::From;
use uuid::Uuid;

mod instance_context;
mod instance_environment;
mod scene;
mod view;
mod view_builder;
mod view_event;

pub use instance_context::*;
pub use instance_environment::*;
pub use scene::*;
pub use view::*;
pub use view_event::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct InstanceId(Uuid);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct ViewId(Uuid);
