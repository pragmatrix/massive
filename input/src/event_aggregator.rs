//! An aggregator that collects events for one specific window and aggregates the cursor position,
//! button states, and keyboard modifiers.
//!
//! Aggregating the scale factor through ScaleFactorChanged does not seem to work on Windows, that
//! event is never sent as of winit 0.22.2.
use std::{collections::HashMap, time::Instant};

use itertools::Itertools;
use massive_geometry::Point;
use winit::{
    event::{ElementState, MouseButton},
    keyboard::ModifiersState,
};

use crate::{AggregationEvent, ButtonSensor, InputEvent};

use super::DeviceId;

#[derive(Debug, Clone, Default)]
pub struct EventAggregator {
    pointing_devices: HashMap<DeviceId, PointingDeviceState>,
    keyboard_modifiers: ModifiersState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationReport {
    /// This is not an event we need to aggregate.
    Ignored,
    /// The event was used to change a state.
    Integrated,
    /// The event is related to the aggregation state, but seems redundant, no aggregate changes
    /// were detected.
    Redundant,
    /// Some prerequisites are not met. For example a mouse button state change was received, but
    /// there was not Position available yet.
    PrerequisitesNotMet,
}

impl EventAggregator {
    pub fn update<E: InputEvent>(&mut self, event: &E, time: Instant) -> AggregationReport {
        let Some(event) = event.to_aggregation_event() else {
            return AggregationReport::Ignored;
        };

        match event {
            AggregationEvent::CursorMoved {
                device_id,
                position,
            } => self.cursor_moved(device_id, position),
            AggregationEvent::CursorEntered { device_id } => self.cursor_entered(device_id),
            AggregationEvent::CursorLeft { device_id } => self.cursor_left(device_id),
            AggregationEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => self.mouse_button_state_changed(time, device_id, button, state),
            AggregationEvent::ModifiersChanged(modifiers) => {
                self.modifiers_changed(modifiers.state())
            }
        }
    }

    fn cursor_entered(&mut self, device_id: DeviceId) -> AggregationReport {
        let device = self.pointing_device_mut(device_id);
        if device.entered {
            return AggregationReport::Redundant;
        }
        device.entered = true;
        AggregationReport::Integrated
    }

    fn cursor_left(&mut self, device_id: DeviceId) -> AggregationReport {
        let device = self.pointing_device_mut(device_id);
        if !device.entered {
            return AggregationReport::Redundant;
        }
        device.entered = false;
        AggregationReport::Integrated
    }

    fn cursor_moved(&mut self, device_id: DeviceId, pos: Point) -> AggregationReport {
        let device_state = self.pointing_device_mut(device_id);
        let pos = Some(pos);
        if device_state.pos == pos {
            return AggregationReport::Redundant;
        }
        device_state.pos = pos;
        AggregationReport::Integrated
    }

    fn mouse_button_state_changed(
        &mut self,
        now: Instant,
        device_id: DeviceId,
        button: MouseButton,
        state: ElementState,
    ) -> AggregationReport {
        let device = self.pointing_device_mut(device_id);
        // If no pos received yet, completely ignore that event.
        // TODO: log this!
        let Some(pos) = device.pos else {
            return AggregationReport::PrerequisitesNotMet;
        };

        device.buttons.insert(
            button,
            MouseButtonState {
                element: state,
                when: now,
                at_pos: pos,
            },
        );

        // Assuming `when` always changes, this is always integrated, even when the same `state` was
        // received.
        AggregationReport::Integrated
    }

    fn modifiers_changed(&mut self, state: ModifiersState) -> AggregationReport {
        if self.keyboard_modifiers == state {
            return AggregationReport::Redundant;
        }

        self.keyboard_modifiers = state;
        AggregationReport::Integrated
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
    #[deprecated(note = "use is_command()")]
    pub fn is_logo(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::SUPER)
    }

    /// `true` if the Windows key on Windows or the Command key on a Mac is pressed.
    pub fn is_command(&self) -> bool {
        self.keyboard_modifiers.contains(ModifiersState::SUPER)
    }

    /// Architecture: May introduce our own modifiers state and add the is_* functions to it?
    pub fn keyboard_modifiers(&self) -> ModifiersState {
        self.keyboard_modifiers
    }

    /// Returns the physical coordinates of the pointing device.
    pub fn pos(&self, id: DeviceId) -> Option<Point> {
        self.pointing_device(id).and_then(|p| p.pos)
    }

    /// When pressed, returns the instant a device was pressed first and where it was pressed.
    // TODO: May introduce a type `PointInTime` or `TimedPoint`, or even `SpaceTime`?
    pub fn is_pressed(&self, sensor: ButtonSensor) -> Option<(Instant, Point)> {
        // TODO implement in terms of `is_pressed_all()`
        let state = self
            .pointing_device(sensor.device)?
            .buttons
            .get(&sensor.button)?;
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

    pub fn pointing_device(&self, id: DeviceId) -> Option<&PointingDeviceState> {
        self.pointing_devices
            .iter()
            .find_map(|(di, s)| (*di == id).then_some(s))
    }
}

#[derive(Clone, Default, Debug)]
pub struct PointingDeviceState {
    pub entered: bool,
    pub pos: Option<Point>,
    pub buttons: HashMap<MouseButton, MouseButtonState>,
}

impl PointingDeviceState {
    pub fn any_button_pressed(&self) -> bool {
        self.buttons
            .values()
            .any(|button_state| button_state.element.is_pressed())
    }
}

#[derive(Debug, Clone)]
pub struct MouseButtonState {
    pub element: ElementState,
    pub when: Instant,
    pub at_pos: Point,
}
