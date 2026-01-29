//! Decides which elements receive which events in which order, so that their view of the input
//! state stays consistent.
//!
//! This type is generic over T, which is the element's type. An target is a reference to a concrete
//! / typed node in the focus and conceptual hierarchy of display elements.

use anyhow::Result;
use derive_more::IntoIterator;
use log::warn;
use winit::event::{DeviceId, ElementState};

use massive_applications::ViewEvent;
use massive_geometry::{Point, Vector3};
use massive_input::Event;

use crate::focus_path::{FocusPath, FocusPathTransition};

#[derive(Debug)]
pub struct EventRouter<T> {
    /// The recently touched target with the cursor / mouse.
    ///
    /// If _any_ button is pressed while moving the cursor, its focus stays on the previous target.
    pointer_focus: FocusPath<T>,

    /// This decides to which view and instance the keyboard events are delivered. Basically the
    /// keyboard focus.
    keyboard_focus: FocusPath<T>,

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

impl<T: PartialEq> Default for EventRouter<T> {
    fn default() -> Self {
        Self {
            pointer_focus: Default::default(),
            keyboard_focus: Default::default(),
            // For now, we assume that _we_ are focused by default but nothing below us.
            outer_focus: OuterFocusState::Focused,
        }
    }
}

impl<T: PartialEq> EventRouter<T>
where
    T: Clone,
{
    pub fn focused(&self) -> &FocusPath<T> {
        &self.keyboard_focus
    }

    pub fn pointer_focus(&self) -> &FocusPath<T> {
        &self.pointer_focus
    }

    /// Change focus to the given path.
    pub fn focus(&mut self, path: FocusPath<T>) -> EventTransitions<T> {
        let mut event_transitions = TransitionLog::new(self.focused().clone());
        set_focus(&mut self.keyboard_focus, path, &mut event_transitions);
        event_transitions.finalize(self.focused().clone())
    }

    pub fn process(
        &mut self,
        input_event: &Event<ViewEvent>,
        hit_tester: &dyn HitTester<T>,
    ) -> Result<EventTransitions<T>> {
        let view_event = input_event.event();

        let mut event_transitions = TransitionLog::new(self.focused().clone());

        let keyboard_focused = &self.keyboard_focus;

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
                    set_pointer_focus(
                        &mut self.pointer_focus,
                        path,
                        *device_id,
                        &mut event_transitions,
                    );
                    Some(hit)
                } else {
                    // Button is pressed, hit directly on the previous target.

                    if let Some(hit) = hit_tester.hit_test_target(screen_pos, &self.pointer_focus) {
                        Some(hit)
                    } else {
                        // No hit on the previous target? This happens if it does not exist anymore,
                        // or some numeric stability problem. In either case, the current cursor
                        // focus must be reset.
                        // Robustness: Shouldn't a regular hit test be attempted?
                        set_pointer_focus(
                            &mut self.pointer_focus,
                            FocusPath::EMPTY,
                            *device_id,
                            &mut event_transitions,
                        );
                        None
                    }
                };

                // If there is a current hit position, forward the event.
                if let Some(hit) = hit {
                    event_transitions.send(
                        &self.pointer_focus,
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
                if *keyboard_focused != self.pointer_focus {
                    set_focus(
                        &mut self.keyboard_focus,
                        self.pointer_focus.clone(),
                        &mut event_transitions,
                    );
                    // Detail: We don't want to forward the event if the focused changed in response
                    // to it, because it would cause a selection to be marked if we animate views in
                    // response to a focus change.
                    //
                    // This would only work when we would move the mouse cursor.
                    //
                    // Feature: We could respond only to a click and let movements get through
                    // without focusing. This way users could select / copy, etc. without moving the
                    // focus?
                } else {
                    event_transitions.send(&self.pointer_focus, view_event.clone());
                }
            }

            // Forward to the current cursor focus.
            //
            // Robustness: We might need to update the cursor
            // focus here again with the current screen pos. The scene might have changed in the
            // meantime.
            ViewEvent::MouseInput { .. } | ViewEvent::MouseWheel { .. } => {
                event_transitions.send(&self.pointer_focus, view_event.clone());
            }

            ViewEvent::CursorEntered { .. } | ViewEvent::CursorLeft { .. } => {}
            ViewEvent::DroppedFile(_) | ViewEvent::HoveredFile(_) => {}

            // Keyboard focus
            ViewEvent::KeyboardInput { .. } | ViewEvent::Ime(..) => {
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

        Ok(event_transitions.finalize(self.focused().clone()))
    }

    fn set_outer_focus(&mut self, focused: bool, transitions: &mut TransitionLog<T>) {
        match (&self.outer_focus, focused) {
            (OuterFocusState::Unfocused { focused_previously }, true) => {
                // Restore focus if nothing is focused.
                //
                // Detail: Focus does not change while the Window is unfocused, see set_foreground.
                if self.keyboard_focus == FocusPath::EMPTY {
                    // Robustness: We may need to check if instances / views are valid here at
                    // the latest, or event better while the Unfocused state is active.
                    set_focus(
                        &mut self.keyboard_focus,
                        focused_previously.clone(),
                        transitions,
                    );
                }
                self.outer_focus = OuterFocusState::Focused
            }
            (OuterFocusState::Focused, false) => {
                // Save and unfocus.
                self.outer_focus = OuterFocusState::Unfocused {
                    focused_previously: self.keyboard_focus.clone(),
                };
                set_focus(&mut self.keyboard_focus, FocusPath::EMPTY, transitions);
            }
            _ => {
                warn!("Redundant Window focus change")
            }
        }
    }
}

fn set_focus<T>(
    focus_path: &mut FocusPath<T>,
    new_path: impl Into<FocusPath<T>>,
    event_transitions: &mut TransitionLog<T>,
) where
    T: PartialEq + Clone,
{
    let focus_transitions = focus_path.transition(new_path.into());
    for transition in focus_transitions {
        let (path, focus) = match transition {
            FocusPathTransition::Exit(path) => (path, false),
            FocusPathTransition::Enter(path) => (path, true),
        };

        // Performance: Recycle path here.
        event_transitions.send(&path, ViewEvent::Focused(focus));
    }
}

fn set_pointer_focus<T>(
    focus_path: &mut FocusPath<T>,
    new_path: impl Into<FocusPath<T>>,
    device_id: DeviceId,
    event_transitions: &mut TransitionLog<T>,
) where
    T: PartialEq + Clone,
{
    let focus_transitions = focus_path.transition(new_path.into());

    for transition in focus_transitions {
        let (path, event) = match transition {
            FocusPathTransition::Exit(path) => (path, ViewEvent::CursorLeft { device_id }),
            FocusPathTransition::Enter(path) => (path, ViewEvent::CursorEntered { device_id }),
        };

        // Performance: Recycle path here.
        event_transitions.send(&path, event);
    }
}

#[must_use]
#[derive(Debug, IntoIterator)]
pub struct EventTransitions<T> {
    #[into_iterator]
    pub transitions: Vec<EventTransition<T>>,
    pub focus_changed: Option<FocusPath<T>>,
}

#[derive(Debug)]
struct TransitionLog<T> {
    transitions: Vec<EventTransition<T>>,
    before_focus: FocusPath<T>,
}

impl<T> TransitionLog<T> {
    fn new(focus: FocusPath<T>) -> Self {
        Self {
            transitions: Vec::new(),
            before_focus: focus,
        }
    }

    fn send(&mut self, path: &FocusPath<T>, event: ViewEvent)
    where
        T: Clone,
    {
        self.transitions
            .push(EventTransition::Directed(path.clone(), event));
    }

    fn broadcast(&mut self, event: ViewEvent) {
        self.transitions.push(EventTransition::Broadcast(event));
    }

    pub fn finalize(self, focus: FocusPath<T>) -> EventTransitions<T>
    where
        T: PartialEq,
    {
        let focus_changed = (self.before_focus != focus).then_some(focus);

        EventTransitions {
            transitions: self.transitions,
            focus_changed,
        }
    }
}

#[derive(Debug)]
pub enum EventTransition<T> {
    Directed(FocusPath<T>, ViewEvent),
    Broadcast(ViewEvent),
}

// Architecture: The two functions can probably be combined into one. But is this a good thing?
pub trait HitTester<Target> {
    /// Return the topmost hit at screen_pos in the target's coordinate system.
    fn hit_test(&self, screen_pos: Point) -> (FocusPath<Target>, Vector3);

    /// Returns the position in the target's coordinate system, even if a regular hit test would
    /// return another target or the point is outside of the hit area.
    fn hit_test_target(&self, screen_pos: Point, target: &FocusPath<Target>) -> Option<Vector3>;
}

#[derive(Debug)]
enum OuterFocusState<T> {
    Unfocused { focused_previously: FocusPath<T> },
    Focused,
}
