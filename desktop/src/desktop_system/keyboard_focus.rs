//! The keyboard focus state (ADR 0021): which target receives keys, what the window focus restores,
//! and where the last typed key went. Like the event router it knows no hierarchy, so its rules
//! stay testable as values; the desktop system applies the changes it returns.

use std::mem;

use log::warn;

use massive_applications::ViewId;

use super::{DesktopTarget, FocusableTarget};
use crate::targeted_event::EventTransition;

#[derive(Debug, Default)]
pub struct KeyboardFocus {
    focused: Option<FocusableTarget>,
    window: WindowFocus,
    input_focus: InputFocus,
}

/// The previous and next keyboard-focused targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardFocusChange {
    pub from: Option<FocusableTarget>,
    pub to: Option<FocusableTarget>,
}

/// Where the last typed key went (ADR 0020). The hover outline marks the keyboard focus only while
/// the user is at the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputFocus {
    /// A desktop level consumed the key, or it was dropped.
    #[default]
    Desktop,
    Application(ViewId),
}

/// The keyboard focus a window focus change asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFocusRequest {
    Clear,
    Restore(FocusableTarget),
}

/// The window focus; everything below the window is unfocused while it is unfocused.
///
/// Architecture: This points to some kind of self similarity. If we would see all the targets as
/// individuals in their role as containers or leaves, it may be possible to avoid managing a focus
/// tree and just manage the focus of the immediate descendants.
#[derive(Debug, Default)]
enum WindowFocus {
    // The window is assumed to start focused.
    #[default]
    Focused,
    Unfocused {
        restore: Option<FocusableTarget>,
    },
}

impl KeyboardFocus {
    pub fn focused(&self) -> Option<FocusableTarget> {
        self.focused
    }

    /// The keyboard focus the hover outline marks: an application that received the last key shows
    /// its own feedback.
    pub fn hover_outline_focus(&self) -> Option<FocusableTarget> {
        self.focused
            .filter(|focused| !self.input_focus.is_application_at(*focused))
    }

    pub fn focus(&mut self, target: Option<FocusableTarget>) -> Option<KeyboardFocusChange> {
        if self.focused == target {
            return None;
        }
        let from = mem::replace(&mut self.focused, target);
        Some(KeyboardFocusChange { from, to: target })
    }

    pub fn set_input_focus(&mut self, input_focus: InputFocus) {
        self.input_focus = input_focus;
    }

    /// Records the window focus and returns the keyboard focus it asks for; `None` if it asks for
    /// no change.
    pub fn set_window_focus(&mut self, focused: bool) -> Option<WindowFocusRequest> {
        match (&self.window, focused) {
            (WindowFocus::Unfocused { restore }, true) => {
                // A focus set while the window was unfocused is newer than the one it lost.
                let request = match (self.focused, *restore) {
                    (Some(_), _) => None,
                    (None, Some(target)) => Some(WindowFocusRequest::Restore(target)),
                    (None, None) => Some(WindowFocusRequest::Clear),
                };
                self.window = WindowFocus::Focused;
                request
            }
            (WindowFocus::Focused, false) => {
                self.window = WindowFocus::Unfocused {
                    restore: self.focused,
                };
                Some(WindowFocusRequest::Clear)
            }
            _ => {
                warn!("Redundant Window focus change");
                None
            }
        }
    }

    /// The focus regaining the window focus restores; `None` while the window is focused.
    pub fn window_restore(&self) -> Option<FocusableTarget> {
        match self.window {
            WindowFocus::Focused => None,
            WindowFocus::Unfocused { restore } => restore,
        }
    }

    /// Retargets the restored focus, so it never refers to a target that left the hierarchy.
    pub fn set_window_restore(&mut self, target: Option<FocusableTarget>) {
        let WindowFocus::Unfocused { restore } = &mut self.window else {
            panic!("The window focus restores a focus only while the window is unfocused");
        };
        *restore = target;
    }

    /// Asserts that no focus state refers to a target that leaves the hierarchy.
    pub fn assert_not_referenced(&self, target: &DesktopTarget) {
        let Some(target) = target.as_focusable() else {
            return;
        };
        assert_ne!(
            self.focused,
            Some(target),
            "Removed target {target:?}, but it had keyboard focus"
        );
        if let WindowFocus::Unfocused { restore } = &self.window {
            assert_ne!(
                *restore,
                Some(target),
                "Removed target {target:?}, but the window focus restores it"
            );
        }
    }
}

impl InputFocus {
    /// The keyboard focus can move without a key, so the application only holds the input focus
    /// while it is still the keyboard-focused view.
    fn is_application_at(&self, keyboard_focus: FocusableTarget) -> bool {
        matches!(
            (self, keyboard_focus),
            (Self::Application(view), FocusableTarget::View(focused)) if *view == focused
        )
    }
}

impl From<KeyboardFocusChange> for EventTransition<DesktopTarget> {
    fn from(change: KeyboardFocusChange) -> Self {
        Self::ChangeKeyboardFocus {
            from: change.from.map(Into::into),
            to: change.to.map(Into::into),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regaining_the_window_focus_restores_the_lost_focus_unless_a_newer_one_is_set() {
        let mut focus = focused_on(FocusableTarget::Desktop);
        focus.set_window_focus(false);
        focus.focus(None);
        assert_eq!(
            focus.set_window_focus(true),
            Some(WindowFocusRequest::Restore(FocusableTarget::Desktop))
        );

        let view = FocusableTarget::View(ViewId::new());
        let mut focus = focused_on(FocusableTarget::Desktop);
        focus.set_window_focus(false);
        focus.focus(Some(view));
        assert_eq!(focus.set_window_focus(true), None);
        assert_eq!(focus.focused(), Some(view));
    }

    #[test]
    fn only_the_application_at_the_focused_view_hides_the_outline() {
        let view = ViewId::new();
        let mut focus = focused_on(FocusableTarget::View(view));

        focus.set_input_focus(InputFocus::Application(ViewId::new()));
        assert_eq!(
            focus.hover_outline_focus(),
            Some(FocusableTarget::View(view))
        );

        focus.set_input_focus(InputFocus::Application(view));
        assert_eq!(focus.hover_outline_focus(), None);
    }

    fn focused_on(target: FocusableTarget) -> KeyboardFocus {
        let mut focus = KeyboardFocus::default();
        focus.focus(Some(target));
        focus
    }
}
