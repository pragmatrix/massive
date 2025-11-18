use derive_more::From;
use uuid::Uuid;

mod instance_client;
mod instance_context;
mod render_target;
mod scene;
mod view;
mod view_builder;

pub use instance_context::*;
pub use render_target::*;
pub use scene::*;
pub use view::*;


#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct InstanceId(Uuid);
