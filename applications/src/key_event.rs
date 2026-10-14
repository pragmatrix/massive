//! The key event views receive. Unlike winit's, whose platform part is private, it can be
//! constructed: the desktop synthesizes releases from it, and tests build events.

use winit::event::{ElementState, KeyEvent as WinitKeyEvent};
use winit::keyboard::{Key, KeyLocation, PhysicalKey, SmolStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub physical_key: PhysicalKey,
    pub logical_key: Key,
    pub text: Option<SmolStr>,
    pub location: KeyLocation,
    pub state: ElementState,
    pub repeat: bool,
    /// The key as the layout produces it without any modifier, which tells whether Option composed
    /// a character; `None` where winit can't tell (web, iOS, Android).
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
