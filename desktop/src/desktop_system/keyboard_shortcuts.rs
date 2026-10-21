//! The keyboard shortcuts of the desktop, recognized from a key press alone.
//!
//! What a shortcut means depends on the level that receives it (ADR 0020).

use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::Direction;
use super::key_input::KeyInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Enter, with any modifiers but Cmd.
    Enter,
    CommandEnter,
    /// Cmd+T
    StartInstance,
    /// Cmd+W without Shift.
    CloseInstance,
    /// Cmd+Shift+W with no other modifier. Only a focused launcher or project consumes it, so
    /// inside an instance the application keeps it.
    RemoveSlot,
    /// Cmd+N with no other modifier, so chords that add one stay free.
    NewLauncher,
    /// Cmd+G with no other modifier.
    Group,
    /// Cmd+Shift+G with no other modifier.
    Ungroup,
    /// Cmd+A with no other modifier.
    SelectAll,
    /// Esc with no modifier.
    Escape,
    /// Cmd+Arrow, and Cmd+Ctrl+Left / Right.
    Navigate(Direction),
    /// Cmd+Shift+Arrow with no other modifier.
    ExtendSelection(Direction),
    /// Cmd+Ctrl+Up
    ZoomIn,
    /// Cmd+Ctrl+Down
    ZoomOut,
}

impl Shortcut {
    pub fn from_input(input: &KeyInput) -> Option<Self> {
        if !input.is_pressed() {
            return None;
        }

        enter(input)
            .or_else(|| escape(input))
            .or_else(|| command_character(input))
            .or_else(|| command_arrow(input))
    }
}

fn enter(input: &KeyInput) -> Option<Shortcut> {
    if input.repeat || input.logical_key != Key::Named(NamedKey::Enter) {
        return None;
    }

    Some(if input.is_command() {
        Shortcut::CommandEnter
    } else {
        Shortcut::Enter
    })
}

fn escape(input: &KeyInput) -> Option<Shortcut> {
    let plain = !input.repeat && input.modifiers.is_empty();
    (plain && input.logical_key == Key::Named(NamedKey::Escape)).then_some(Shortcut::Escape)
}

fn command_character(input: &KeyInput) -> Option<Shortcut> {
    if input.repeat || !input.is_command() {
        return None;
    }

    match &input.logical_key {
        Key::Character(c) => match c.as_str() {
            "t" | "T" => Some(Shortcut::StartInstance),
            "w" if !input.is_shift() => Some(Shortcut::CloseInstance),
            "w" | "W" if input.modifiers == ModifiersState::SUPER | ModifiersState::SHIFT => {
                Some(Shortcut::RemoveSlot)
            }
            "n" if input.modifiers == ModifiersState::SUPER => Some(Shortcut::NewLauncher),
            "g" if input.modifiers == ModifiersState::SUPER => Some(Shortcut::Group),
            "g" | "G" if input.modifiers == ModifiersState::SUPER | ModifiersState::SHIFT => {
                Some(Shortcut::Ungroup)
            }
            "a" if input.modifiers == ModifiersState::SUPER => Some(Shortcut::SelectAll),
            _ => None,
        },
        _ => None,
    }
}

/// Unlike the other shortcuts, holding an arrow key repeats.
fn command_arrow(input: &KeyInput) -> Option<Shortcut> {
    if !input.is_command() {
        return None;
    }

    let direction = match &input.logical_key {
        Key::Named(NamedKey::ArrowLeft) => Direction::Left,
        Key::Named(NamedKey::ArrowRight) => Direction::Right,
        Key::Named(NamedKey::ArrowUp) => Direction::Up,
        Key::Named(NamedKey::ArrowDown) => Direction::Down,
        _ => return None,
    };

    Some(match direction {
        Direction::Up if input.is_ctrl() => Shortcut::ZoomIn,
        Direction::Down if input.is_ctrl() => Shortcut::ZoomOut,
        direction if input.modifiers == ModifiersState::SUPER | ModifiersState::SHIFT => {
            Shortcut::ExtendSelection(direction)
        }
        direction => Shortcut::Navigate(direction),
    })
}

#[cfg(test)]
mod tests {
    use winit::event::ElementState;

    use super::*;

    const COMMAND: ModifiersState = ModifiersState::SUPER;

    fn input(key: Key, state: ElementState, repeat: bool, modifiers: ModifiersState) -> KeyInput {
        KeyInput::new(key, state, repeat, modifiers)
    }

    #[test]
    fn only_presses_are_shortcuts_and_only_arrows_repeat() {
        let enter = Key::Named(NamedKey::Enter);
        let left = Key::Named(NamedKey::ArrowLeft);

        let release =
            |key| Shortcut::from_input(&input(key, ElementState::Released, false, COMMAND));
        let repeated =
            |key| Shortcut::from_input(&input(key, ElementState::Pressed, true, COMMAND));

        assert_eq!(release(enter.clone()), None);
        assert_eq!(repeated(enter), None);
        assert_eq!(repeated(Key::Character("t".into())), None);
        assert_eq!(repeated(left), Some(Shortcut::Navigate(Direction::Left)));
    }

    /// Shift turns a navigation into a selection, but only without further modifiers.
    #[test]
    fn shift_cmd_arrows_extend_the_selection() {
        let right = |modifiers| {
            Shortcut::from_input(&input(
                Key::Named(NamedKey::ArrowRight),
                ElementState::Pressed,
                true,
                modifiers,
            ))
        };

        assert_eq!(
            right(COMMAND | ModifiersState::SHIFT),
            Some(Shortcut::ExtendSelection(Direction::Right))
        );
        assert_ne!(
            right(COMMAND | ModifiersState::SHIFT | ModifiersState::ALT),
            Some(Shortcut::ExtendSelection(Direction::Right))
        );
    }
}
