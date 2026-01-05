mod application_registry;
mod desktop_environment;
mod desktop_presenter;
pub(crate) mod desktop;
mod event_router;
mod focus_tree;
mod instance_manager;
mod projects;
mod desktop_ui;

pub use application_registry::Application;
pub use desktop_environment::*;
pub use event_router::{EventRouter, EventTransition, HitTester};
pub use desktop_ui::*;
