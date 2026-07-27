use derive_more::From;
use uuid::Uuid;

mod frame;
mod instance_context;
mod instance_environment;
mod project;
mod view;
mod view_builder;
mod view_event;

pub use frame::*;
pub use instance_context::*;
pub use instance_environment::*;
pub use project::*;
pub use view::*;
pub use view_event::*;

pub use massive_scene::Scene;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct InstanceId(Uuid);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct ViewId(Uuid);
