pub mod shell;
pub use shell::{ApplicationContext, ShellWindow, WindowRenderer};

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}
