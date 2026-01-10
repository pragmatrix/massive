mod application_registry;
pub(crate) mod desktop;
mod desktop_environment;
mod desktop_presenter;
mod desktop_ui;
mod event_router;
mod focus_tree;
mod instance_manager;
mod projects;

pub use application_registry::Application;
pub use desktop::Desktop;
pub use desktop_environment::*;
pub use desktop_ui::*;
pub use event_router::{EventRouter, EventTransition, HitTester};
