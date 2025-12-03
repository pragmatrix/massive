pub mod application_context;
pub mod async_window_renderer;
pub mod shell;
pub mod shell_window;
pub mod window_renderer;
mod window_renderer_builder;

pub use application_context::ApplicationContext;
pub use async_window_renderer::*;
// pub use font_system_builder::FontSystemBuilder;
pub use massive_applications::Scene;
pub use shell::ShellEvent;
pub use shell_window::ShellWindow;
pub use window_renderer::WindowRenderer;
pub use window_renderer_builder::WindowRendererBuilder;

pub use massive_renderer::{FontId, FontManager, FontWeight};

// Re-exports to make life easier for shell users.
pub use anyhow::Result;

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}
