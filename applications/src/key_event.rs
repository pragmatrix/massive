//! The key event views receive. Unlike winit's, whose platform part is private, it can be
//! constructed: the desktop synthesizes releases from it, and tests build events.

use winit::event::{ElementState, KeyEvent as WinitKeyEvent};
use winit::keyboard::{Key, KeyLocation, PhysicalKey, SmolStr};

/// Its `Debug` output leaves out which key it is and what it types, because key events carry what
/// the user types, passwords at a prompt without echo included (`docs/security.md`).
#[derive(derive_more::Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    #[debug(skip)]
    pub physical_key: PhysicalKey,
    #[debug(skip)]
    pub logical_key: Key,
    #[debug(skip)]
    pub text: Option<SmolStr>,
    pub location: KeyLocation,
    pub state: ElementState,
    pub repeat: bool,
    /// The key as the layout produces it without any modifier, which tells whether Option composed
    /// a character; `None` where winit can't tell (web, iOS, Android).
    #[debug(skip)]
    pub key_without_modifiers: Option<Key>,
}

impl From<&WinitKeyEvent> for KeyEvent {
    fn from(event: &WinitKeyEvent) -> Self {
        Self {
            physical_key: event.physical_key,
            logical_key: event.logical_key.clone(),
            text: event.text.clone(),
            location: event.location,
            state: event.state,
            repeat: event.repeat,
            key_without_modifiers: key_without_modifiers(event),
        }
    }
}

/// winit supplements key events with the unmodified key on these platforms.
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "redox"
))]
fn key_without_modifiers(event: &WinitKeyEvent) -> Option<Key> {
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    Some(event.key_without_modifiers())
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "redox"
)))]
fn key_without_modifiers(_event: &WinitKeyEvent) -> Option<Key> {
    None
}

#[cfg(test)]
mod tests {
    use winit::event::Ime;
    use winit::keyboard::{KeyCode, PhysicalKey};

    use super::*;
    use crate::ViewEvent;

    /// Logs print events with `Debug`, so its output must not show what the user typed.
    #[test]
    fn debug_output_leaves_out_typed_text() {
        let key = KeyEvent {
            physical_key: PhysicalKey::Code(KeyCode::KeyQ),
            logical_key: Key::Character("§".into()),
            text: Some("§".into()),
            location: KeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            key_without_modifiers: Some(Key::Character("§".into())),
        };
        let ime = ViewEvent::Ime(Ime::Commit("secret".into()));
        let output = format!("{key:?} {ime:?}");
        assert!(
            !output.contains('§') && !output.contains("KeyQ") && !output.contains("secret"),
            "{output}"
        );
        assert!(
            output.contains("Pressed") && output.contains("Commit"),
            "{output}"
        );
    }
}
