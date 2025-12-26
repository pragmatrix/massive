//! Decides which elements receive which events in which order, so that their view of the input
//! state stays consistent.
//!
//! This type is generic over T, which is the element's type. An target is a reference to a concrete
//! / typed node in the focus and conceptual hierarchy of display elements.

use anyhow::Result;
use derive_more::{From, Into};
use log::warn;
use winit::event::ElementState;

use massive_applications::ViewEvent;
use massive_geometry::{Point, Vector3};
use massive_input::Event;

use crate::focus_tree::{FocusPath, FocusTransition, FocusTransitions, FocusTree};

#[derive(Debug)]
pub struct EventRouter<T> {
    /// The recently touched target with the cursor / mouse.
    ///
    /// If _any_ button is pressed while moving the cursor, its focus stays on the previous target.
    cursor_focus: FocusPath<T>,

    /// This decides to which view and instance the keyboard events are delivered. Basically the
    /// keyboard focus.
    focus_tree: FocusTree<T>,

    /// The current state of the outer focus state (perhaps the Window).
    ///
    /// This is used to remember the previously focused path, because we do unfocus everything in
    /// the focus tree.
    ///
    /// Architecture: May be the focus tree should do that?
    ///
    /// Architecture: This points so some kind of self similarity, if we would see all the targets
    /// as individuals in their role as containers or leaves, it may be possible to avoid managing a
    /// focus tree here and just manage the focus of the immediate descendants.
    outer_focus: OuterFocusState<T>,
}

impl<T> Default for EventRouter<T> {
    fn default() -> Self {
        Self {
            cursor_focus: Default::default(),
            focus_tree: Default::default(),
            // For now, we assume that _we_ are focused by default but nothing below us.
            outer_focus: OuterFocusState::Focused,
        }
    }
}

impl<T: PartialEq> EventRouter<T>
where
    T: Clone,
{
    /// Change focus to the given path.
    pub fn focus(&mut self, path: FocusPath<T>) -> EventTransitions<T> {
        let mut event_transitions = EventTransitions::default();
        set_focus(&mut self.focus_tree, path, &mut event_transitions);
        event_transitions
    }

    pub fn focused(&self) -> &FocusPath<T> {
        self.focus_tree.focused()
    }

    pub fn handle_event(
        &mut self,
        input_event: &Event<ViewEvent>,
        hit_tester: &dyn HitTester<T>,
    ) -> Result<EventTransitions<T>> {
        let view_event = input_event.event();

        let mut event_transitions = EventTransitions::default();

        let keyboard_focused = self.focus_tree.focused();

        match view_event {
            ViewEvent::Focused(focused) => {
                self.set_outer_focus(*focused, &mut event_transitions);
            }

            ViewEvent::CursorMoved { device_id, .. } => {
                let any_pressed = input_event
                    .pointing_device_state(*device_id)
                    .map(|d| d.any_button_pressed())
                    .unwrap_or(false);

                let screen_pos = input_event
                    .device_pos(*device_id)
                    .expect("Internal error: A CursorMoved event must have set a position");

                // Change the cursor focus only if there is no button pressed.
                //
                // Robustness: There might be a change of the device here.
                let hit = if !any_pressed {
                    let (path, hit) = hit_tester.hit_test(screen_pos);
                    // Feature: Need to consider CursorExit / Enter messages here.
                    self.cursor_focus = path;
                    Some(hit)
                } else {
                    // Button is pressed, hit directly on the previous target.

                    if let Some(hit) = hit_tester.hit_test_target(screen_pos, &self.cursor_focus) {
                        Some(hit)
                    } else {
                        // No hit on the previous target? This happens if it does not exist anymore,
                        // or some numeric stability problem. In either case, the current cursor
                        // focus must be reset.
                        // Robustness: Shouldn't a regular hit test be attempted?
                        self.cursor_focus = FocusPath::EMPTY;
                        None
                    }
                };

                // If there is a current hit position, forward the event.
                if let Some(hit) = hit {
                    event_transitions.send(
                        &self.cursor_focus,
                        ViewEvent::CursorMoved {
                            device_id: *device_id,
                            position: (hit.x, hit.y),
                        },
                    );
                }
            }

            // Handle a mouse button press. This may cause a focus change.
            ViewEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => {
                if *keyboard_focused != self.cursor_focus {
                    set_focus(
                        &mut self.focus_tree,
                        self.cursor_focus.clone(),
                        &mut event_transitions,
                    );
                }

                event_transitions.send(&self.cursor_focus, view_event.clone());
            }

            // Forward to the current cursor focus.
            //
            // Robustness: We might need to update the cursor
            // focus here again with the current screen pos. The scene might have changed in the
            // meantime.
            ViewEvent::MouseInput { .. } | ViewEvent::MouseWheel { .. } => {
                event_transitions.send(&self.cursor_focus, view_event.clone());
            }

            ViewEvent::CursorEntered { .. } | ViewEvent::CursorLeft { .. } => {}
            ViewEvent::DroppedFile(_) | ViewEvent::HoveredFile(_) => {}

            // Keyboard focus
            ViewEvent::KeyboardInput { .. } | ViewEvent::Ime(..) => {
                // if key_event.state == ElementState::Pressed
                //     && !key_event.repeat
                //     && input_event.states().is_command()
                //     && let Some(ViewPath { instance, .. }) = focused_view
                // {
                //     match &key_event.logical_key {
                //         Key::Character(c) if c.as_str() == "t" => {
                //             let application = instance_manager.get_application_name(instance)?;
                //             return Ok(UiCommand::StartInstance {
                //                 application: application.to_string(),
                //                 originating_instance: instance,
                //             });
                //         }
                //         Key::Character(c) if c.as_str() == "w" => {
                //             return Ok(UiCommand::StopInstance { instance });
                //         }
                //         _ => {}
                //     }
                // }

                event_transitions.send(keyboard_focused, view_event.clone());
            }

            // All views
            ViewEvent::ModifiersChanged(_) => {
                event_transitions.broadcast(view_event.clone());
            }

            ViewEvent::HoveredFileCancelled | ViewEvent::CloseRequested => {}

            // Robustness: Figure out how to handle these.
            ViewEvent::Resized(_) => {}
        }

        Ok(event_transitions)
    }

    fn set_outer_focus(&mut self, focused: bool, transitions: &mut EventTransitions<T>) {
        match (&self.outer_focus, focused) {
            (OuterFocusState::Unfocused { focused_previously }, true) => {
                // Restore focus if nothing is focused.
                //
                // Detail: Focus does not change while the Window is unfocused, see set_foreground.
                if *self.focus_tree.focused() == FocusPath::EMPTY {
                    // Robustness: We may need to check if instances / views are valid here at
                    // the latest, or event better while the Unfocused state is active.
                    set_focus(
                        &mut self.focus_tree,
                        focused_previously.clone(),
                        transitions,
                    );
                }
                self.outer_focus = OuterFocusState::Focused
            }
            (OuterFocusState::Focused, false) => {
                // Save and unfocus.
                self.outer_focus = OuterFocusState::Unfocused {
                    focused_previously: self.focus_tree.focused().clone(),
                };
                set_focus(&mut self.focus_tree, FocusPath::EMPTY, transitions);
            }
            _ => {
                warn!("Redundant Window focus change")
            }
        }
    }
}

fn set_focus<T>(
    focus_tree: &mut FocusTree<T>,
    path: impl Into<FocusPath<T>>,
    event_transitions: &mut EventTransitions<T>,
) where
    T: PartialEq + Clone,
{
    let path = path.into();
    let focus_transitions = focus_tree.focus(path);
    forward_focus_transitions(focus_transitions, event_transitions);
}

fn forward_focus_transitions<T>(
    focus_transitions: FocusTransitions<T>,
    event_transitions: &mut EventTransitions<T>,
) where
    T: Clone,
{
    for transition in focus_transitions {
        let (path, focus) = match transition {
            FocusTransition::Exit(path) => (path, false),
            FocusTransition::Enter(path) => (path, true),
        };

        // Performance: Recycle path here.
        event_transitions.send(&path, ViewEvent::Focused(focus));
    }
}

#[derive(Debug, From, Into)]
pub struct EventTransitions<T>(Vec<EventTransition<T>>);

impl<T> Default for EventTransitions<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> EventTransitions<T> {
    fn send(&mut self, path: &FocusPath<T>, event: ViewEvent)
    where
        T: Clone,
    {
        self.0.push(EventTransition::Send(path.clone(), event));
    }

    fn broadcast(&mut self, event: ViewEvent) {
        self.0.push(EventTransition::Broadcast(event));
    }

    pub fn into_vec(self) -> Vec<EventTransition<T>> {
        self.0
    }
}

#[derive(Debug)]
pub enum EventTransition<T> {
    Send(FocusPath<T>, ViewEvent),
    Broadcast(ViewEvent),
}

pub trait HitTester<T> {
    /// Return the topmost hist at screen_pos.
    fn hit_test(&self, screen_pos: Point) -> (FocusPath<T>, Vector3);
    /// Returns the position transformed on target, even if it's outside of it.
    fn hit_test_target(&self, screen_pos: Point, target: &FocusPath<T>) -> Option<Vector3>;
}

#[derive(Debug)]
enum OuterFocusState<T> {
    Unfocused { focused_previously: FocusPath<T> },
    Focused,
}
