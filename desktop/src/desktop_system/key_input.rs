use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use massive_applications::ViewEvent;

/// A key event reduced to what key handlers decide on, plus the event to hand to an application.
#[derive(Debug)]
pub struct KeyInput {
    pub logical_key: Key,
    pub state: ElementState,
    pub repeat: bool,
    pub modifiers: ModifiersState,
    view_event: ViewEvent,
}

impl KeyInput {
    #[cfg(test)]
    pub fn new(
        logical_key: Key,
        state: ElementState,
        repeat: bool,
        modifiers: ModifiersState,
    ) -> Self {
        Self {
            logical_key,
            state,
            repeat,
            modifiers,
            // Handlers in tests never forward it.
            view_event: ViewEvent::Focused(true),
        }
    }

    /// `None` if `view_event` is not a key event.
    pub fn from_view_event(view_event: &ViewEvent, modifiers: ModifiersState) -> Option<Self> {
        let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = view_event
        else {
            return None;
        };

        Some(Self {
            logical_key: key_event.logical_key.clone(),
            state: key_event.state,
            repeat: key_event.repeat,
            modifiers,
            view_event: view_event.clone(),
        })
    }

    /// A fresh press of a key that is not a modifier. Modifiers alter pointer interaction (Cmd
    /// activates hyperlinks), so they are not typing.
    pub fn is_typed(&self) -> bool {
        self.is_pressed() && !self.repeat && !Self::is_modifier_key(&self.logical_key)
    }

    pub fn is_pressed(&self) -> bool {
        self.state == ElementState::Pressed
    }

    pub fn is_modifier_key(key: &Key) -> bool {
        matches!(
            key,
            Key::Named(
                NamedKey::Super
                    | NamedKey::Meta
                    | NamedKey::Shift
                    | NamedKey::Control
                    | NamedKey::Alt
            )
        )
    }

    /// `true` if the Windows key on Windows or the Command key on a Mac is pressed.
    pub fn is_command(&self) -> bool {
        self.modifiers.super_key()
    }

    pub fn is_shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    /// The event as an application receives it.
    pub fn view_event(&self) -> &ViewEvent {
        &self.view_event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_non_modifier_press_is_typed() {
        let key = |key, state, repeat| KeyInput::new(key, state, repeat, ModifiersState::empty());
        let character = || Key::Character("a".into());
        let cmd = || Key::Named(NamedKey::Super);

        assert!(key(character(), ElementState::Pressed, false).is_typed());
        assert!(!key(character(), ElementState::Pressed, true).is_typed());
        assert!(!key(character(), ElementState::Released, false).is_typed());
        assert!(!key(cmd(), ElementState::Pressed, false).is_typed());
    }
}
