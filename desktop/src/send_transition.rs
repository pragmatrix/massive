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
                    || ViewEvent::Focused(false),
                    || ViewEvent::Focused(true),
                    path_resolver,
                ));
            }
            EventTransition::ChangePointerFocus { from, to } => {
                send_transitions.extend(focus_change_to_send_transitions(
                    from.as_ref(),
                    to.as_ref(),
                    keyboard_modifiers,
                    || ViewEvent::CursorLeft,
                    || ViewEvent::CursorEntered,
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
    on_exit: impl Fn() -> ViewEvent,
    on_enter: impl Fn() -> ViewEvent,
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
                let event = on_exit();
                send_transitions.push(SendTransition(target, event));
            }
            FocusTransition::Enter(target) => {
                let event = on_enter();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrderedHierarchy;

    fn transition_signature(transitions: Vec<SendTransition<i32>>) -> Vec<(i32, &'static str)> {
        transitions
            .into_iter()
            .map(|SendTransition(target, event)| {
                let kind = match event {
                    ViewEvent::Focused(true) => "FocusIn",
                    ViewEvent::Focused(false) => "FocusOut",
                    ViewEvent::CursorEntered => "CursorEntered",
                    ViewEvent::CursorLeft => "CursorLeft",
                    ViewEvent::ModifiersChanged(_) => "ModifiersChanged",
                    _ => "Other",
                };
                (target, kind)
            })
            .collect()
    }

    fn tree_with_shared_root() -> OrderedHierarchy<i32> {
        let mut hierarchy = OrderedHierarchy::default();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(2, 3).unwrap();
        hierarchy.add(1, 4).unwrap();
        hierarchy.add(4, 5).unwrap();
        hierarchy
    }

    #[test]
    fn keyboard_focus_change_adds_enter_exit_and_modifiers_tail() {
        let hierarchy = tree_with_shared_root();
        let transitions = convert_to_send_transitions(
            [EventTransition::ChangeKeyboardFocus {
                from: Some(3),
                to: Some(5),
            }],
            Modifiers::default(),
            &hierarchy,
        );

        assert_eq!(
            transition_signature(transitions),
            vec![
                (3, "FocusOut"),
                (2, "FocusOut"),
                (4, "FocusIn"),
                (5, "FocusIn"),
                (5, "ModifiersChanged"),
            ]
        );
    }

    #[test]
    fn pointer_focus_change_also_includes_modifiers_tail() {
        let hierarchy = tree_with_shared_root();
        let transitions = convert_to_send_transitions(
            [EventTransition::ChangePointerFocus {
                from: Some(3),
                to: Some(5),
            }],
            Modifiers::default(),
            &hierarchy,
        );

        assert_eq!(
            transition_signature(transitions),
            vec![
                (3, "CursorLeft"),
                (2, "CursorLeft"),
                (4, "CursorEntered"),
                (5, "CursorEntered"),
                (5, "ModifiersChanged"),
            ]
        );
    }

    #[test]
    fn none_to_none_focus_change_produces_no_transitions() {
        let hierarchy = tree_with_shared_root();
        let transitions = convert_to_send_transitions(
            [EventTransition::ChangeKeyboardFocus {
                from: None,
                to: None,
            }],
            Modifiers::default(),
            &hierarchy,
        );

        assert!(transitions.is_empty());
    }
}
