//! An aggregator that collects events for one specific window and aggregates the cursor position,
//! button states, and keyboard modifiers.
//!
//! Aggregating the scale factor through ScaleFactorChanged does not seem to work on Windows, that
//! event is never sent as of winit 0.22.2.
use std::{collections::HashMap, time::Instant};

use itertools::Itertools;
use massive_geometry::Point;
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    keyboard::ModifiersState,
};

use super::{DeviceId, ExternalEvent};

#[derive(Debug, Clone, Default)]
pub struct EventAggregator {
    pointing_devices: HashMap<DeviceId, PointingDeviceState>,
    keyboard_modifiers: ModifiersState,
}

impl EventAggregator {
    pub fn update(&mut self, event: &ExternalEvent, to_logical: impl Fn(Point) -> Point) {
        if let ExternalEvent::Window {
            ref event, time, ..
        } = *event
        {
            match *event {
                WindowEvent::CursorMoved {
                    device_id,
                    position,
                    ..
                } => self.cursor_moved(device_id, to_logical((position.x, position.y).into())),
                WindowEvent::CursorEntered { device_id } => self.cursor_entered(device_id),
                WindowEvent::CursorLeft { device_id } => {
                    self.cursor_left(device_id);
                }
                WindowEvent::MouseInput {
                    device_id,
                    state,
                    button,
                    ..
                } => self.mouse_button_state_changed(time, device_id, button, state),
                WindowEvent::ModifiersChanged(modifiers) => {
                    self.modifiers_changed(modifiers.state())
                }
                _ => {}
            }
        }
    }

    fn cursor_entered(&mut self, device_id: DeviceId) {
        self.pointing_device_mut(device_id).entered = true;
    }

    fn cursor_left(&mut self, device_id: DeviceId) {
        self.pointing_device_mut(device_id).entered = false;
    }

    fn cursor_moved(&mut self, device_id: DeviceId, pos: Point) {
        self.pointing_device_mut(device_id).pos = Some(pos);
    }

    fn mouse_button_state_changed(
        &mut self,
        now: Instant,
        device_id: DeviceId,
        button: MouseButton,
        state: ElementState,
    ) {
        let device = self.pointing_device_mut(device_id);
        // If no pos received yet, completely ignore that event.
        // TODO: log this!
        if let Some(pos) = device.pos {
            device.buttons.insert(
                button,
                MouseButtonState {
                    element: state,
                    when: now,
                    at_pos: pos,
                },
            );
        }
    }

    fn modifiers_changed(&mut self, state: ModifiersState) {
        self.keyboard_modifiers = state
    }

    fn pointing_device_mut(&mut self, device_id: DeviceId) -> &mut PointingDeviceState {
        self.pointing_devices.entry(device_id).or_default()
    }

    pub fn to_device_states(&self) -> DeviceStates {
        let devices: Vec<_> = self
            .pointing_devices
            .iter()
            .map(|(a, b)| (*a, b.clone()))
            .collect();

        DeviceStates {
            pointing_devices: devices,
            keyboard_modifiers: self.keyboard_modifiers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceStates {
    pointing_devices: Vec<(DeviceId, PointingDeviceState)>,
    keyboard_modifiers: ModifiersState,
}

#[derive(Clone, Default, Debug)]
struct PointingDeviceState {
    entered: bool,
    pos: Option<Point>,
    buttons: HashMap<MouseButton, MouseButtonState>,
}

#[derive(Debug, Clone)]
struct MouseButtonState {
    element: ElementState,
    when: Instant,
    at_pos: Point,
}

impl DeviceStates {
    /// `true` if the Shift button is pressed.
    pub fn is_shift(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::SHIFT)
    }

    /// `true` if the Ctrl button is pressed.
    pub fn is_ctrl(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::CONTROL)
    }

    /// `true` if the Alt button is pressed.
    pub fn is_alt(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::ALT)
    }

    /// `true` if the Windows key on Windows or the Command key on a Mac is pressed.
    pub fn is_logo(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::SUPER)
    }

    /// Returns the logical window coordinates of the pointing device.
    pub fn pos(&self, id: DeviceId) -> Option<Point> {
        self.pointing_device(id).and_then(|p| p.pos)
    }

    /// When pressed, returns the instant a device was pressed first and where it was pressed.
    // TODO: May introduce a type `PointInTime` or `TimedPoint`, or even `SpaceTime`?
    pub fn is_pressed(&self, device_id: DeviceId, button: MouseButton) -> Option<(Instant, Point)> {
        // TODO implement in terms of `is_pressed_all()`
        let state = self.pointing_device(device_id)?.buttons.get(&button)?;
        (state.element == ElementState::Pressed).then_some((state.when, state.at_pos))
    }

    /// Returns all the devices that have the given [`MouseButton`] pressed. Most recent presses
    /// first.
    pub fn all_pressed(
        &self,
        button: MouseButton,
    ) -> impl Iterator<Item = (DeviceId, (Instant, Point))> + '_ {
        self.pointing_devices
            .iter()
            .flat_map(move |(id, state)| {
                let state = state.buttons.get(&button)?;
                (state.element == ElementState::Pressed)
                    .then_some((*id, (state.when, state.at_pos)))
            })
            .sorted_by_key(|(_, (instant, _))| *instant)
            .rev()
    }

    fn pointing_device(&self, id: DeviceId) -> Option<&PointingDeviceState> {
        self.pointing_devices
            .iter()
            .find_map(|(di, s)| (*di == id).then_some(s))
    }
}
