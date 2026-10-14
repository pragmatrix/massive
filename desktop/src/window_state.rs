use derive_more::Constructor;

use winit::window::CursorIcon;

use massive_applications::ViewId;
use massive_geometry::{BoxPx, SizePx};
use massive_shell::ShellWindow;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Constructor)]
pub struct WindowState {
    pub inner_size: SizePx,
    pub is_fullscreen: bool,
}

#[derive(Debug, Clone)]
pub struct WindowPresentationState {
    pub title: String,
    pub cursor_visible: bool,
    pub cursor: CursorIcon,
    /// The view the IME composes for; `None` disables it. Bound to the view, so a composition
    /// never continues in another one.
    pub ime_view: Option<ViewId>,
    /// Where macOS places its text input UI (IME candidates, caps lock indicator), in surface
    /// pixels. `None` keeps the last position.
    pub input_area: Option<BoxPx>,
}

impl Default for WindowPresentationState {
    fn default() -> Self {
        Self {
            title: "".into(),
            // Usually, when the Window opens, the cursor is visible.
            cursor_visible: true,
            cursor: CursorIcon::default(),
            ime_view: None,
            input_area: None,
        }
    }
}

impl WindowPresentationState {
    pub fn sync(&mut self, new_state: WindowPresentationState, window: &ShellWindow) {
        if self.title != new_state.title {
            window.set_title(&new_state.title);
        }
        if self.cursor_visible != new_state.cursor_visible {
            window.set_cursor_visible(new_state.cursor_visible);
        }
        if self.cursor != new_state.cursor {
            window.set_cursor(new_state.cursor);
        }
        if self.ime_view != new_state.ime_view {
            // Disallowing discards the composition pending in the view that lost the focus.
            if self.ime_view.is_some() {
                window.set_ime_allowed(false);
            }
            if new_state.ime_view.is_some() {
                window.set_ime_allowed(true);
            }
        }
        if self.input_area != new_state.input_area
            && let Some(area) = new_state.input_area
        {
            window.set_ime_cursor_area(area);
        }

        *self = new_state
    }
}
