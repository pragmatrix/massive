//! Decides which elements receive which events in which order, so that their view of the input
//! state stays consistent.
//!
//! This type is generic over `T`, which is the element's type. A target is a reference to a concrete
//! / typed node in the focus and conceptual hierarchy of display elements.

use std::{fmt, mem};

use anyhow::{Result, bail};
use log::{error, warn};

use winit::event::{DeviceId, ElementState, Modifiers};

use massive_applications::ViewEvent;
use massive_geometry::{Point, Vector3};
use massive_input::{DeviceStates, Event};

use crate::desktop_system::KeyInput;
use crate::policy::constants::pointer_feedback;

// Detail: The EventRouter works without any knowledge about the relationships between the targets
// (e.g. their hierarchical structure).
#[derive(Debug)]
pub struct EventRouter<T> {
    /// The recently touched target with the cursor / mouse.
    ///
    /// If _any_ button is pressed while moving the cursor, its focus stays on the previous target.
    pointer_focus: Option<PointerFocusTarget<T>>,

    /// Most recent [`DeviceStates`]. This way we can re-hit the pointer anytime.
    device_states: DeviceStates,
}

/// Ordered routing decisions for one input event.
#[derive(Debug)]
pub struct RouterOutput<T> {
    pub steps: Vec<RouterStep<T>>,
}

/// One routing decision; the output order preserves focus and delivery ordering.
///
/// The keyboard focus is kept beside the router (ADR 0021), so steps for it carry no target.
#[derive(Debug)]
pub enum RouterStep<T> {
    PointerFocusChanged(PointerFocusChange<T>),
    /// A pointer button was pressed on the pointer focus; `None` when the press has no target.
    PointerPressed(Option<T>),
    /// The window gained or lost focus.
    WindowFocusChanged(bool),
    /// Keys, IME input, and modifier changes, addressed to the keyboard focus.
    KeyboardInput(ViewEvent),
    DeliverInput {
        target: T,
        event: ViewEvent,
    },
}

/// The previous and next pointer-focus owners.
#[derive(Debug)]
pub struct PointerFocusChange<T> {
    pub from: Option<PointerFocusTarget<T>>,
    pub to: Option<PointerFocusTarget<T>>,
}

/// A target and device pair identifying the current pointer focus.
#[derive(Debug, Clone, PartialEq)]
pub struct PointerFocusTarget<T> {
    pub target: T,
    pub device_id: DeviceId,
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
            device_states: Default::default(),
        }
    }

    /// Internal function to check if there are dangling references.
    pub fn notify_removed(&self, target: &T) -> Result<()> {
        if self.pointer_focus() == Some(target) {
            bail!("Removed target {target:?}, but it hat pointer focus");
        }
        Ok(())
    }

    pub fn pointer_focus(&self) -> Option<&T> {
        self.pointer_focus.as_ref().map(|focus| &focus.target)
    }

    /// The event pointer's last position, absent while keyboard input suppresses pointer feedback.
    pub fn pointer_position(&self) -> Option<Point> {
        let focus = self.pointer_focus.as_ref()?;
        self.device_states.pos(focus.device_id)
    }

    pub fn keyboard_modifiers(&self) -> Modifiers {
        self.device_states.keyboard_modifiers()
    }

    pub fn any_buttons_pressed(&self) -> bool {
        self.device_states.any_buttons_pressed()
    }

    pub fn process(
        &mut self,
        input_event: &Event<ViewEvent>,
        hit_tester: &impl HitTest<T>,
    ) -> Result<RouterOutput<T>> {
        let view_event = input_event.event();

        let mut steps = Vec::new();

        match view_event {
            ViewEvent::Focused(focused) => {
                steps.push(RouterStep::WindowFocusChanged(*focused));
            }

            ViewEvent::CursorMoved {
                device_id,
                position: _,
            } => {
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
                let hit_pos = if !any_pressed {
                    if self.pointer_focus.is_none() {
                        // Pointer feedback is currently suppressed (i.e. the focus was cleared for
                        // keyboard navigation). Plain cursor motion is ignored.
                        if input_event.cursor_has_velocity(
                            pointer_feedback::REENABLE_MIN_DISTANCE_PX,
                            pointer_feedback::REENABLE_MAX_DURATION,
                        ) &&
                            // Cursor movement velocity goes over a certain threshold, re-hit and
                            // re-enable cursor focus.
                            let Some((target, hit_pos)) = hit_tester.hit_test(screen_pos, None)
                        {
                            if let Some(change) = self.set_pointer_focus(Some(PointerFocusTarget {
                                target,
                                device_id: *device_id,
                            })) {
                                steps.push(RouterStep::PointerFocusChanged(change));
                            }
                            Some(hit_pos)
                        } else {
                            None
                        }
                    } else if let Some((target, hit_pos)) = hit_tester.hit_test(screen_pos, None) {
                        if let Some(change) = self.set_pointer_focus(Some(PointerFocusTarget {
                            target,
                            device_id: *device_id,
                        })) {
                            steps.push(RouterStep::PointerFocusChanged(change));
                        }
                        Some(hit_pos)
                    } else {
                        // Hit test should always hit Desktop at least, so this branch may never
                        // enter (may be hit_test() should cover this).
                        error!("Internal Error: Unexpected hit test result");
                        if let Some(change) = self.set_pointer_focus(None) {
                            steps.push(RouterStep::PointerFocusChanged(change));
                        }
                        None
                    }
                } else {
                    // Button is pressed, hit directly on the previous target if there is one.

                    if let Some((_, hit)) =
                        // Robustness: What if pointer_focus is root?
                        hit_tester.hit_test(
                            screen_pos,
                            self.pointer_focus.as_ref().map(|focus| &focus.target),
                        )
                    {
                        Some(hit)
                    } else {
                        // No hit on the previous target? This happens if it does not exist anymore,
                        // or some numeric stability problem. In either case, the current cursor
                        // focus must be reset.
                        // Robustness: Shouldn't a regular hit test be attempted?
                        warn!("Resetting pointer focus, no hit on previous target");
                        if let Some(change) = self.set_pointer_focus(None) {
                            steps.push(RouterStep::PointerFocusChanged(change));
                        }
                        None
                    }
                };

                // If there is a current hit position & pointer focus, forward the event.
                if let (Some(hit_pos), Some(focused)) =
                    (hit_pos, &self.pointer_focus)
                    // Keep devices with pressed buttons from moving another device's focus.
                    && focused.device_id == *device_id
                {
                    steps.push(RouterStep::DeliverInput {
                        target: focused.target.clone(),
                        event: ViewEvent::CursorMoved {
                            device_id: focused.device_id,
                            position: (hit_pos.x, hit_pos.y).into(),
                        },
                    });
                }
            }

            // Handle a mouse button press. This may cause a focus change of the pointer and
            // keyboard focus.
            ViewEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                ..
            } => {
                // Detail: We do forward the event if the focused changed in response to it, even
                // though is might cause an accidental selection if the camera moves in response to
                // a click.
                //
                // To get around this, the system must make sure that the camera does not move while
                // a button is pressed.
                //
                // Geometry can move without pointer motion, so presses re-hit-test.
                if let Some(screen_pos) = input_event
                    .device_pos(*device_id)
                    .or_else(|| self.device_states.pos(*device_id))
                {
                    let target = hit_tester
                        .hit_test(screen_pos, None)
                        .map(|(target, _)| (target, *device_id));
                    if let Some(change) = self.set_pointer_focus(
                        target.map(|(target, device_id)| PointerFocusTarget { target, device_id }),
                    ) {
                        steps.push(RouterStep::PointerFocusChanged(change));
                    }
                }

                let pressed_target = self
                    .pointer_focus
                    .as_ref()
                    .filter(|focus| focus.device_id == *device_id)
                    .map(|focus| focus.target.clone());
                steps.push(RouterStep::PointerPressed(pressed_target.clone()));
                if let Some(target) = pressed_target {
                    steps.push(RouterStep::DeliverInput {
                        target,
                        event: view_event.clone(),
                    });
                }
            }

            // Forward to the current pointer focus.
            //
            // Robustness: We might need to update the pointer focus here again with the current
            // screen position. The scene might have changed in the meantime.
            ViewEvent::MouseInput { device_id, .. } => {
                // If pointer focus is not set, re-set it if the hit tester says so.
                if self.pointer_focus.is_none()
                    && let Some(change) =
                        self.hit_test_and_set_pointer_focus(hit_tester, *device_id)?
                {
                    steps.push(RouterStep::PointerFocusChanged(change));
                }

                if let Some(pointer_focus) = &self.pointer_focus
                    && pointer_focus.device_id == *device_id
                {
                    steps.push(RouterStep::DeliverInput {
                        target: pointer_focus.target.clone(),
                        event: view_event.clone(),
                    });
                }
            }

            ViewEvent::MouseWheel { device_id, .. } => {
                if let Some(pointer_focus) = &self.pointer_focus {
                    if pointer_focus.device_id == *device_id {
                        steps.push(RouterStep::DeliverInput {
                            target: pointer_focus.target.clone(),
                            event: view_event.clone(),
                        });
                    }
                } else if let Some(screen_pos) = input_event
                    .device_pos(*device_id)
                    .or_else(|| self.device_states.pos(*device_id))
                    && let Some((target, _)) = hit_tester.hit_test(screen_pos, None)
                {
                    // Scrolling reaches the surface under the pointer without reenabling hover.
                    steps.push(RouterStep::DeliverInput {
                        target,
                        event: view_event.clone(),
                    });
                }
            }

            ViewEvent::CursorEntered { .. } | ViewEvent::CursorLeft { .. } => {}
            ViewEvent::DroppedFile(_) | ViewEvent::HoveredFile(_) => {}

            // Keyboard focus
            ViewEvent::KeyboardInput { event, .. } => {
                steps.push(RouterStep::KeyboardInput(view_event.clone()));

                // Unfocus the cursor when a key is newly pressed. Modifier keys are excluded, they
                // are used to alter pointer interaction (for example Cmd to activate hyperlinks).
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && !KeyInput::is_modifier_key(&event.logical_key)
                    && let Some(change) = self.set_pointer_focus(None)
                {
                    steps.push(RouterStep::PointerFocusChanged(change));
                }
            }

            ViewEvent::Ime(..) | ViewEvent::ModifiersChanged(_) => {
                steps.push(RouterStep::KeyboardInput(view_event.clone()));
            }

            ViewEvent::HoveredFileCancelled => {}

            // Desktop handles close before input routing; it is not focus-targeted.
            ViewEvent::CloseRequested => {}

            ViewEvent::Occluded(_) => {}

            // Robustness: Figure out how to handle these.
            ViewEvent::RedrawRequested | ViewEvent::Resized(_) => {}
        }

        // Commit device states.
        self.device_states = input_event.device_states().clone();

        Ok(RouterOutput { steps })
    }

    /// The pointer focus should be tested again with hit-testing against all targets.
    ///
    /// Robustness: There is perhaps a need to send a `CursorMove` event to the newly hit target,
    /// otherwise the current position may be off?
    pub fn hit_test_and_set_pointer_focus(
        &mut self,
        hit_tester: &dyn HitTest<T>,
        device_id: DeviceId,
    ) -> Result<Option<PointerFocusChange<T>>> {
        let target = {
            // This is somehow a shortcut. We just check for the latest Device's position change.
            // Robustness: Support multiple pointers.
            if let Some(pos) = self.device_states.pos(device_id) {
                if let Some((target, _hit)) = hit_tester.hit_test(pos, None) {
                    Some(target)
                } else {
                    // Robustness: No hit -> No target, is this even correct?
                    None
                }
            } else {
                warn!("Resetting pointer focus: No most recent position was found");
                if self.pointer_focus.is_none() {
                    return Ok(None);
                }
                bail!(
                    "Internal error: Pointer focus was set, but no most recent position was found"
                );
            }
        };

        Ok(self.set_pointer_focus(target.map(|target| PointerFocusTarget { target, device_id })))
    }

    pub fn unfocus_pointer(&mut self) -> Option<PointerFocusChange<T>> {
        self.set_pointer_focus(None)
    }

    fn set_pointer_focus(
        &mut self,
        new_focus: Option<PointerFocusTarget<T>>,
    ) -> Option<PointerFocusChange<T>> {
        if self.pointer_focus == new_focus {
            return None;
        }

        let from = mem::replace(&mut self.pointer_focus, new_focus.clone());
        Some(PointerFocusChange {
            from,
            to: new_focus,
        })
    }
}

// Architecture: The two functions can probably be combined into one. But is this a good thing?
pub trait HitTest<Target> {
    /// Return the topmost hit at screen_pos in the target's coordinate system.
    ///
    /// If target is set, returns the hit inside the specific Target's coordinate system only.
    fn hit_test(&self, screen_pos: Point, target: Option<&Target>) -> Option<(Target, Vector3)>;
}
