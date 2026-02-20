use massive_applications::ViewEvent;
use winit::event::Modifiers;

use crate::EventTransition;
use crate::focus_path::{FocusTransition, PathResolver};

#[derive(Debug)]
pub struct SendTransition<T>(pub T, pub ViewEvent);

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
                send_transitions.push(SendTransition(target, event));
            }
            EventTransition::ChangeKeyboardFocus { from, to } => {
                send_transitions.extend(focus_change_to_send_transitions(
                    from.as_ref(),
                    to.as_ref(),
                    keyboard_modifiers,
                    |_| ViewEvent::Focused(false),
                    |_| ViewEvent::Focused(true),
                    path_resolver,
                ));
            }
            EventTransition::ChangePointerFocus {
                from,
                to,
                device_id,
            } => {
                send_transitions.extend(focus_change_to_send_transitions(
                    from.as_ref(),
                    to.as_ref(),
                    keyboard_modifiers,
                    |_| ViewEvent::CursorLeft { device_id },
                    |_| ViewEvent::CursorEntered { device_id },
                    path_resolver,
                ));
            }
        }
    }

    send_transitions
}

fn focus_change_to_send_transitions<T>(
    from: Option<&T>,
    to: Option<&T>,
    modifiers: Modifiers,
    on_exit: impl Fn(&T) -> ViewEvent,
    on_enter: impl Fn(&T) -> ViewEvent,
    path_resolver: &impl PathResolver<T>,
) -> Vec<SendTransition<T>>
where
    T: Clone + PartialEq,
{
    let from_path = path_resolver.resolve_path(from);
    let to_path = path_resolver.resolve_path(to);

    let mut send_transitions = Vec::new();
    for transition in from_path.transitions(to_path) {
        match transition {
            FocusTransition::Exit(target) => {
                let event = on_exit(&target);
                send_transitions.push(SendTransition(target, event));
            }
            FocusTransition::Enter(target) => {
                let event = on_enter(&target);
                send_transitions.push(SendTransition(target, event));
            }
        }
    }

    if let Some(to) = to {
        send_transitions.push(SendTransition(
            to.clone(),
            ViewEvent::ModifiersChanged(modifiers),
        ));
    }

    send_transitions
}
