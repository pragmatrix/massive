#[cfg(target_os = "macos")]
mod macos_menu;

use winit::event_loop::EventLoopProxy;

use crate::shell::ShellCommand;

pub(crate) fn initialize_platform_menu(proxy: EventLoopProxy<ShellCommand>) {
    #[cfg(target_os = "macos")]
    macos_menu::initialize_platform_menu(proxy);
    #[cfg(not(target_os = "macos"))]
    let _ = proxy;
}

pub(crate) fn toggle_fullscreen() {
    #[cfg(target_os = "macos")]
    macos_menu::toggle_fullscreen();
}
