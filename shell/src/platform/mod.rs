#[cfg(target_os = "macos")]
mod macos_menu;

pub(crate) fn initialize_platform_menu() {
    #[cfg(target_os = "macos")]
    macos_menu::initialize_platform_menu();
}
