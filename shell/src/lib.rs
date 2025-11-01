pub mod application_context;
pub mod async_window_renderer;
mod message_filter;
mod scene;
pub mod shell;
pub mod shell_window;
pub mod window_renderer;

pub use application_context::ApplicationContext;
pub use async_window_renderer::*;
pub use scene::Scene;
pub use shell::ShellEvent;
pub use shell_window::ShellWindow;
pub use window_renderer::WindowRenderer;

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}
