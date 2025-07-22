pub mod async_window_renderer;
pub mod shell;
pub mod shell_window;
pub mod window_renderer;

pub use async_window_renderer::AsyncWindowRenderer;
pub use shell::ApplicationContext;
pub use shell_window::ShellWindow;
pub use window_renderer::WindowRenderer;

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}
