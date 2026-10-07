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

/// Raise the calling thread to the user-interactive quality-of-service class (macOS), so the
/// scheduler keeps it on performance cores and does not delay it behind background work.
///
/// Does nothing unless the `interactive-thread-priority` feature is enabled.
pub(crate) fn set_current_thread_user_interactive(name: &str) {
    #[cfg(all(target_os = "macos", feature = "interactive-thread-priority"))]
    {
        // SAFETY: Only affects the calling thread.
        let result = unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
        };
        if result == 0 {
            log::info!("{name}: QOS_CLASS_USER_INTERACTIVE set");
        } else {
            log::warn!("{name}: setting QOS_CLASS_USER_INTERACTIVE failed with {result}");
        }
    }
    #[cfg(not(all(target_os = "macos", feature = "interactive-thread-priority")))]
    let _ = name;
}
