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
    /// `None` disables the IME.
    pub ime: Option<WindowIme>,
}

/// The IME is bound to the focused view, so a composition never continues in another view.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct WindowIme {
    pub view: ViewId,
    /// The input area in surface pixels; `None` while the view is not visible, which keeps the
    /// last position.
    pub area: Option<BoxPx>,
}

impl Default for WindowPresentationState {
    fn default() -> Self {
        Self {
            title: "".into(),
            // Usually, when the Window opens, the cursor is visible.
            cursor_visible: true,
            cursor: CursorIcon::default(),
            ime: None,
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
        if self.ime != new_state.ime {
            Self::sync_ime(self.ime, new_state.ime, window);
        }

        *self = new_state
    }

    fn sync_ime(current: Option<WindowIme>, new: Option<WindowIme>, window: &ShellWindow) {
        if current.map(|ime| ime.view) != new.map(|ime| ime.view) {
            // Disallowing discards the composition pending in the view that lost the focus.
            if current.is_some() {
                window.set_ime_allowed(false);
            }
            if new.is_some() {
                window.set_ime_allowed(true);
            }
        }
        if let Some(area) = new.and_then(|ime| ime.area) {
            window.set_ime_cursor_area(area);
        }
    }
}
