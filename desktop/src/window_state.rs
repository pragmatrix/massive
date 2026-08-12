use derive_more::Constructor;

use winit::window::CursorIcon;

use massive_geometry::SizePx;
use massive_shell::ShellWindow;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Constructor)]
pub struct WindowState {
    pub inner_size: SizePx,
    pub is_fullscreen: bool,
}

impl WindowState {
    pub fn from_window(window: &ShellWindow) -> Self {
        Self {
            inner_size: window.inner_size(),
            is_fullscreen: window.is_fullscreen(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowPresentationState {
    // It's fine that the cursor is not visible in the default state.
    pub title: String,
    pub cursor_visible: bool,
    pub cursor: CursorIcon,
}

impl Default for WindowPresentationState {
    fn default() -> Self {
        Self {
            title: "".into(),
            cursor_visible: true,
            cursor: CursorIcon::default(),
        }
    }
}

impl WindowPresentationState {
    pub fn delta_sync(&mut self, new_state: WindowPresentationState, window: &ShellWindow) {
        if self.title != new_state.title {
            window.set_title(&new_state.title);
        }
        if self.cursor_visible != new_state.cursor_visible {
            window.set_cursor_visible(new_state.cursor_visible);
        }
        if self.cursor != new_state.cursor {
            window.set_cursor(new_state.cursor);
        }

        *self = new_state
    }
}
