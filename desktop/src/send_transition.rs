use massive_applications::ViewEvent;
use winit::event::{DeviceId, Modifiers};

use crate::EventTransition;
use crate::focus_path::{FocusTransition, PathResolver};

#[derive(Debug)]
pub enum SendTransition<T> {
    Send(T, ViewEvent),
}

pub fn convert_to_send_transitions<T>(
    transitions: impl IntoIterator<Item = EventTransition<T>>,
    keyboard_modifiers: Modifiers,
    path_resolver: &impl PathResolver<T>,
) -> Vec<SendTransition<T>>
where
    T: Clone + PartialEq,
{
    let mut send_transitions = Vec::new();

    for transition in transitions {
        match transition {
            EventTransition::Send(target, event) => {
                send_transitions.push(SendTransition::Send(target, event));
            }
            EventTransition::ChangeKeyboardFocus { from, to } => {
                send_transitions.extend(focus_change_to_send_transitions(
                    from,
                    to,
                    keyboard_modifiers,
                    path_resolver,
                ));
            }
            EventTransition::ChangePointerFocus {
                from,
                to,
                device_id,
            } => {
                send_transitions.extend(pointer_focus_change_to_send_transitions(
                    from,
                    to,
                    device_id,
                    path_resolver,
                ));
            }
        }
    }

    send_transitions
}

fn focus_change_to_send_transitions<T>(
    from: T,
    to: T,
    keyboard_modifiers: Modifiers,
    path_resolver: &impl PathResolver<T>,
) -> Vec<SendTransition<T>>
where
    T: Clone + PartialEq,
{
    let from_path = path_resolver.resolve_path(&from);
    let to_path = path_resolver.resolve_path(&to);

    let mut send_transitions = Vec::new();
    for transition in from_path.transitions(to_path) {
        match transition {
            FocusTransition::Exit(target) => {
                send_transitions.push(SendTransition::Send(target, ViewEvent::Focused(false)));
            }
            FocusTransition::Enter(target) => {
                send_transitions.push(SendTransition::Send(target, ViewEvent::Focused(true)));
            }
        }
    }

    send_transitions.push(SendTransition::Send(
        to,
        ViewEvent::ModifiersChanged(keyboard_modifiers),
    ));

    send_transitions
}

fn pointer_focus_change_to_send_transitions<T>(
    from: T,
    to: T,
    device_id: DeviceId,
    path_resolver: &impl PathResolver<T>,
) -> Vec<SendTransition<T>>
where
    T: Clone + PartialEq,
{
    let from_path = path_resolver.resolve_path(&from);
    let to_path = path_resolver.resolve_path(&to);

    let mut send_transitions = Vec::new();
    for transition in from_path.transitions(to_path) {
        match transition {
            FocusTransition::Exit(target) => {
                send_transitions.push(SendTransition::Send(
                    target,
                    ViewEvent::CursorLeft { device_id },
                ));
            }
            FocusTransition::Enter(target) => {
                send_transitions.push(SendTransition::Send(
                    target,
                    ViewEvent::CursorEntered { device_id },
                ));
            }
        }
    }

    send_transitions
}
