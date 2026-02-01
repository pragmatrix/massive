mod application_registry;
mod band_presenter;
pub(crate) mod desktop;
mod desktop_environment;
mod desktop_interaction;
mod desktop_presenter;
mod event_router;
mod focus_path;
mod focus_target;
mod instance_manager;
mod instance_presenter;
mod navigation;
mod projects;

pub use application_registry::Application;
pub use desktop::Desktop;
pub use desktop_environment::*;
pub use desktop_interaction::*;
pub use desktop_presenter::DesktopPresenter;
pub use event_router::{EventRouter, EventTransition, HitTester};


// A layout helper.
// Robustness: Can't we implement ToPixels somewhere?

pub fn box_to_rect(([x, y], [w, h]): massive_layout::Box<2>) -> massive_geometry::RectPx {
    massive_geometry::RectPx::new((x, y).into(), (w as i32, h as i32).into())
}
