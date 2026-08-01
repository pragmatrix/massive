use derive_more::From;
use uuid::Uuid;

mod application_event;
mod frame;
mod instance_context;
mod instance_environment;
mod project;
mod view;
mod view_builder;
mod view_event;

pub use application_event::*;
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

impl ViewId {
	pub fn new() -> Self {
		Uuid::new_v4().into()
	}
}

/// Identifies a shell presentation clock.
///
/// This is neither a [`ViewId`], which identifies logical application content and input targets,
/// nor a native window identifier. A presentation may be backed by a window, an embedded surface,
/// or another host-specific rendering target.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub struct PresentationId(Uuid);

impl PresentationId {
	pub fn new() -> Self {
		Uuid::new_v4().into()
	}
}
