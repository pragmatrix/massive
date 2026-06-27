//! Decides which elements receive which events in which order, so that their view of the input
//! state stays consistent.
//!
//! This type is generic over `T`, which is the element's type. A target is a reference to a concrete
//! / typed node in the focus and conceptual hierarchy of display elements.

use std::fmt;

use anyhow::{Result, bail};
use derive_more::IntoIterator;
use log::warn;
use winit::event::{DeviceId, ElementState, Modifiers};

use massive_applications::ViewEvent;
use massive_geometry::{Point, Vector3};
use massive_input::{DeviceStates, Event};

#[derive(Debug)]
pub struct NavigationTarget<T> {
    pub target: T,
    pub event: Option<ViewEvent>,
}

// Detail: The EventRouter works without any knowledge about the relationships between the targets
// (e.g. their hierarchical structure).
#[derive(Debug)]
pub struct EventRouter<T> {
    /// The recently touched target with the cursor / mouse.
    ///
    /// If _any_ button is pressed while moving the cursor, its focus stays on the previous target.
    pointer_focus: Option<T>,

    /// The keyboard focus decides to which view and instance the keyboard events are delivered.
    keyboard_focus: Option<T>,

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

    /// Most recent [`DeviceStates`]. This way we can re-hit the pointer anytime.
    device_states: DeviceStates,
}

impl<T: PartialEq + Clone + fmt::Debug> Default for EventRouter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> EventRouter<T>
where
    T: Clone + fmt::Debug + PartialEq,
{
    pub fn new() -> Self {
        Self {
            // Shouldn't we wait until the pointer actually move here (should this be optional).
            pointer_focus: None,
            keyboard_focus: None,
            // For now, we assume that _we_ are focused by default but nothing below us.
            outer_focus: OuterFocusState::Focused,
            device_states: Default::default(),
        }
    }

    /// Internal function to check if there are dangling references.
    pub fn notify_removed(&self, target: &T) -> Result<()> {
        if self.keyboard_focus.as_ref() == Some(target) {
            bail!("Removed target {target:?}, but it had keyboard focus");
        }
        if self.pointer_focus.as_ref() == Some(target) {
            bail!("Removed target {target:?}, but it hat pointer focus");
        }

        if let OuterFocusState::Unfocused { focused_previously } = &self.outer_focus
            && focused_previously.as_ref() == Some(target)
        {
            bail!("Removed target {target:?}, but it had captured in the outer focus");
        }

        Ok(())
    }

    pub fn focused(&self) -> Option<&T> {
        self.keyboard_focus.as_ref()
    }

    pub fn pointer_focus(&self) -> Option<&T> {
        self.pointer_focus.as_ref()
    }

    pub fn keyboard_modifiers(&self) -> Modifiers {
        self.device_states.keyboard_modifiers()
    }

    pub fn any_buttons_pressed(&self) -> bool {
        self.device_states.any_buttons_pressed()
    }

    /// Change focus to the given target.
    pub fn focus<'a>(&mut self, focus: impl Into<Option<&'a T>>) -> EventTransitions<T>
    where
        T: 'a,
    {
        let mut event_transitions = EventTransitions::default();
        self.set_keyboard_focus(focus.into().cloned(), &mut event_transitions);
        event_transitions
    }

    pub fn process(
        &mut self,
        input_event: &Event<ViewEvent>,
        hit_tester: &impl HitTester<T>,
    ) -> Result<ProcessOutcome<T>> {
        let view_event = input_event.event();

        let mut event_transitions = EventTransitions::default();
        let mut focus_outcome = None;

        match view_event {
            ViewEvent::Focused(focused) => {
                if let Some(target) = self.set_outer_focus(*focused) {
                    focus_outcome = Some(ProcessOutcome::Focus(target.map(|target| {
                        NavigationTarget {
                            target,
                            event: None,
                        }
                    })));
                }
            }

            ViewEvent::CursorMoved { .. } => {
                let any_pressed = input_event
                    .pointing_device_state(DeviceId::dummy())
                    .map(|d| d.any_button_pressed())
                    .unwrap_or(false);

                let screen_pos = input_event
                    .device_pos(DeviceId::dummy())
                    .expect("Internal error: A CursorMoved event must have set a position");

                // Change the cursor focus only if there is no button pressed.
                //
                // Robustness: There might be a change of the device here.
                let hit = if !any_pressed {
                    if let Some((target, hit)) = hit_tester.hit_test(screen_pos, None) {
                        self.set_pointer_focus(Some(target), &mut event_transitions);
                        Some(hit)
                    } else {
                        self.set_pointer_focus(None, &mut event_transitions);
                        None
                    }
                } else {
                    // Button is pressed, hit directly on the previous target if there is one.

                    if let Some((_, hit)) =
                        // Robustness: What if pointer_focus is root?
                        hit_tester.hit_test(screen_pos, self.pointer_focus.as_ref())
                    {
                        Some(hit)
                    } else {
                        // No hit on the previous target? This happens if it does not exist anymore,
                        // or some numeric stability problem. In either case, the current cursor
                        // focus must be reset.
                        // Robustness: Shouldn't a regular hit test be attempted?
                        warn!("Resetting pointer focus, no hit on previous target");
                        self.set_pointer_focus(None, &mut event_transitions);
                        None
                    }
                };

                // If there is a current hit position, forward the event.
                if let Some(hit) = hit
                    && let Some(focused) = &self.pointer_focus
                {
                    event_transitions.send(focused, ViewEvent::CursorMoved((hit.x, hit.y).into()));
                }
            }

            // Handle a mouse button press. This may cause a focus change.
            ViewEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => {
                // Detail: We do forward the event if the focused changed in response to it, even
                // though is might cause an accidental selection if the camera moves in response to
                // a click.
                //
                // To get around this, the system must make sure that the camera does not move while
                // a button is pressed.
                if let Some(pointer_focus) = &self.pointer_focus {
                    focus_outcome = Some(ProcessOutcome::Focus(Some(NavigationTarget {
                        target: pointer_focus.clone(),
                        event: Some(view_event.clone()),
                    })));
                } else if self.keyboard_focus.is_some() {
                    focus_outcome = Some(ProcessOutcome::Focus(None));
                }
            }

            // Forward to the current pointer focus.
            //
            // Robustness: We might need to update the pointer focus here again with the current
            // screen position. The scene might have changed in the meantime.
            ViewEvent::MouseInput { .. } | ViewEvent::MouseWheel { .. } => {
                if let Some(pointer_focus) = &self.pointer_focus {
                    event_transitions.send(pointer_focus, view_event.clone());
                }
            }

            ViewEvent::CursorEntered | ViewEvent::CursorLeft => {}
            ViewEvent::DroppedFile(_) | ViewEvent::HoveredFile(_) => {}

            // Keyboard focus
            ViewEvent::KeyboardInput { .. } | ViewEvent::Ime(..) => {
                if let Some(keyboard_focus) = &self.keyboard_focus {
                    event_transitions.send(keyboard_focus, view_event.clone());
                }
            }

            ViewEvent::ModifiersChanged(_) => {
                // Robustness: Not sure if this is the right call, we send modifiers changed to
                // both, pointer focused _and_ the keyboard focused.
                if let Some(keyboard_focus) = &self.keyboard_focus {
                    event_transitions.send(keyboard_focus, view_event.clone());
                }
                if let Some(pointer_focus) = &self.pointer_focus
                    && self.pointer_focus != self.keyboard_focus
                {
                    event_transitions.send(pointer_focus, view_event.clone());
                }
            }

            ViewEvent::HoveredFileCancelled | ViewEvent::CloseRequested => {}

            // Robustness: Figure out how to handle these.
            ViewEvent::Resized(_) => {}
        }

        // Commit device states.
        self.device_states = input_event.device_states().clone();

        if let Some(outcome) = focus_outcome {
            return Ok(outcome);
        }

        Ok(ProcessOutcome::Transitions(event_transitions))
    }

    /// The pointer focus should be tested again with hit-testing against all targets.
    ///
    /// Robustness: There is perhaps a need to send a `CursorMove` event to the newly hit target,
    /// otherwise the current position may be off?
    pub fn reset_pointer_focus(
        &mut self,
        hit_tester: &dyn HitTester<T>,
    ) -> Result<EventTransitions<T>> {
        let target = {
            // This is somehow a shortcut. We just check for the latest Device's position change.
            // Robustness: Support multiple pointers.
            if let Some(pos) = self.device_states.pos(DeviceId::dummy()) {
                if let Some((target, _hit)) = hit_tester.hit_test(pos, None) {
                    Some(target)
                } else {
                    // Robustness: No hit -> No target, is this even correct?
                    None
                }
            } else {
                warn!("Resetting pointer focus: No most recent position was found");
                if self.pointer_focus.is_none() {
                    return Ok(Default::default());
                }
                bail!(
                    "Internal error: Pointer focus was set, but no most recent position was found"
                );
            }
        };

        // We don't need a focus change tracking here.
        let mut transitions = EventTransitions::default();
        self.set_pointer_focus(target, &mut transitions);
        Ok(transitions)
    }

    pub fn unfocus_pointer(&mut self) -> Result<EventTransitions<T>> {
        let mut transitions = EventTransitions::default();
        self.set_pointer_focus(None, &mut transitions);
        Ok(transitions)
    }

    /// Updates outer (window-level) focus state and returns an optional keyboard-focus suggestion.
    ///
    /// Return value meaning:
    /// - `None`: no keyboard-focus change is suggested (redundant outer-focus event).
    /// - `Some(None)`: clear keyboard focus.
    /// - `Some(Some(target))`: focus the given target.
    fn set_outer_focus(&mut self, focused: bool) -> Option<Option<T>> {
        match (&self.outer_focus, focused) {
            (OuterFocusState::Unfocused { focused_previously }, true) => {
                // Restore focus if nothing is focused.
                //
                // Detail: Focus does not change while the Window is unfocused, see set_foreground.
                let focus_target = if self.keyboard_focus.is_none() {
                    focused_previously.clone()
                } else {
                    None
                };

                // Robustness: We may need to check if instances / views are valid here at
                // the latest, or event better while the Unfocused state is active.

                self.outer_focus = OuterFocusState::Focused;
                Some(focus_target)
            }
            (OuterFocusState::Focused, false) => {
                // Save and unfocus.
                self.outer_focus = OuterFocusState::Unfocused {
                    focused_previously: self.keyboard_focus.clone(),
                };
                // Robustness: What about pointer focus?
                Some(None)
            }
            _ => {
                warn!("Redundant Window focus change");
                None
            }
        }
    }

    fn set_keyboard_focus(&mut self, new: Option<T>, transitions: &mut EventTransitions<T>) {
        if self.keyboard_focus == new {
            return;
        }

        // Idea: Can't this be completely event-sourced, isn't the current state just a reflection of
        // the events?
        transitions.add(EventTransition::ChangeKeyboardFocus {
            from: self.keyboard_focus.clone(),
            to: new.clone(),
        });

        // Commit
        self.keyboard_focus = new;
    }

    fn set_pointer_focus(&mut self, new: Option<T>, transitions: &mut EventTransitions<T>) {
        if self.pointer_focus == new {
            return;
        }

        transitions.add(EventTransition::ChangePointerFocus {
            from: self.pointer_focus.clone(),
            to: new.clone(),
        });

        // Commit
        self.pointer_focus = new;
    }
}

#[derive(Debug)]
pub enum ProcessOutcome<T> {
    Transitions(EventTransitions<T>),
    Focus(Option<NavigationTarget<T>>),
}

#[must_use]
#[derive(Debug, IntoIterator)]
pub struct EventTransitions<T>(Vec<EventTransition<T>>);

impl<T> Default for EventTransitions<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> EventTransitions<T> {
    pub fn targets_affected_by_keyboard_focus_change(&self) -> Vec<&T> {
        let mut touched = Vec::new();

        for transition in &self.0 {
            if let EventTransition::ChangeKeyboardFocus { from, to } = transition {
                if let Some(from) = from.as_ref() {
                    touched.push(from);
                }
                if let Some(to) = to.as_ref() {
                    touched.push(to);
                }
            }
        }

        touched
    }

    fn send(&mut self, target: &T, event: ViewEvent)
    where
        T: Clone,
    {
        self.add(EventTransition::Send(target.clone(), event));
    }

    fn add(&mut self, transition: EventTransition<T>) {
        self.0.push(transition);
    }
}

#[derive(Debug)]
pub enum EventTransition<T> {
    // Send a targeted event.
    Send(T, ViewEvent),
    ChangePointerFocus { from: Option<T>, to: Option<T> },
    ChangeKeyboardFocus { from: Option<T>, to: Option<T> },
}

// Architecture: The two functions can probably be combined into one. But is this a good thing?
pub trait HitTester<Target> {
    /// Return the topmost hit at screen_pos in the target's coordinate system.
    ///
    /// If target is set, returns the hit inside the specific Target's coordinate system only.
    fn hit_test(&self, screen_pos: Point, target: Option<&Target>) -> Option<(Target, Vector3)>;
}

#[derive(Debug)]
enum OuterFocusState<T> {
    Unfocused { focused_previously: Option<T> },
    Focused,
}
