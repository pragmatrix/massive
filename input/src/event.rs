use std::fmt;
use std::time::{Duration, Instant};

use winit::event::{DeviceId, ElementState, Modifiers, MouseButton};

use massive_geometry::{Point, Vector};
use winit::keyboard::ModifiersState;

use crate::event_aggregator::DeviceStates;
use crate::event_history::{EventHistory, EventRecord};
use crate::tracker::Movement;
use crate::{AggregationEvent, ButtonSensor, InputEvent, MouseGesture, PointingDeviceState};

#[derive(Clone)]
pub struct Event<'history, E: InputEvent> {
    /// Gorilla: The event history, including the most recent event which represents this event.
    history: &'history EventHistory<E>,
}

// Custom Debug implementation, so that we don't print the full history.
impl<E: InputEvent> fmt::Debug for Event<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.record().fmt(f)
    }
}

impl<'history, E: InputEvent> Event<'history, E> {
    pub fn new(history: &'history EventHistory<E>) -> Self {
        assert!(history.current().is_some());
        Self { history }
    }

    pub fn pressed(&self, sensor: ButtonSensor) -> bool {
        matches!(self.to_aggregation_event(), Some(AggregationEvent::MouseInput {
                device_id, state, ..
            }) if device_id == sensor.device && state == ElementState::Pressed)
    }

    pub fn released(&self, sensor: ButtonSensor) -> bool {
        matches!(self.to_aggregation_event(), Some(AggregationEvent::MouseInput {
                device_id, state, ..
            }) if device_id == sensor.device && state == ElementState::Released)
    }

    /// Returns the physical coordinates if the event was caused by a pointer device and the device
    /// has reported a position yet.
    ///
    // Robustness: I think we should make this require the device() to be passed, this is otherwise
    pub fn pos(&self) -> Option<Point> {
        self.device_states().pos(self.device()?)
    }

    pub fn device_pos(&self, device: DeviceId) -> Option<Point> {
        self.device_states().pos(device)
    }

    /// Returns the device that is associated with the event.
    pub fn device(&self) -> Option<DeviceId> {
        self.event().device()
    }

    pub fn mouse_pressed(&self) -> Option<ButtonSensor> {
        self.button_sensor_and_state()
            .filter(|(_, state)| *state == ElementState::Pressed)
            .map(|(sensor, _)| sensor)
    }

    pub fn mouse_released(&self) -> Option<ButtonSensor> {
        self.button_sensor_and_state()
            .filter(|(_, state)| *state == ElementState::Released)
            .map(|(sensor, _)| sensor)
    }

    /// The usage here answer more the question: Any gestures active?
    pub fn any_buttons_pressed(&self) -> bool {
        self.record().states.any_buttons_pressed()
    }

    /// If this is a mouse button event, return its sensor and state.
    pub fn button_sensor_and_state(&self) -> Option<(ButtonSensor, ElementState)> {
        match self.event().to_aggregation_event() {
            Some(AggregationEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            }) => Some((ButtonSensor::new(device_id, button), state)),
            _ => None,
        }
    }

    /// Returns the [`DeviceId`] of the event if this is a
    /// [`winit::event::WindowEvent::CursorMoved`] event.
    pub fn cursor_moved(&self) -> Option<DeviceId> {
        match self.to_aggregation_event()? {
            AggregationEvent::CursorMoved { device_id, .. } => Some(device_id),
            _ => None,
        }
    }

    pub fn detect_click(&self, mouse_button: MouseButton) -> Option<Point> {
        match self.to_aggregation_event()? {
            AggregationEvent::MouseInput { state, button, .. }
                if button == mouse_button && state == ElementState::Pressed =>
            {
                Some(self.pos().unwrap())
            }
            _ => None,
        }
    }

    // This return the point where the mouse button was released, this way users can undo the click.
    pub fn detect_clicked(&self, mouse_button: MouseButton) -> Option<Point> {
        match self.to_aggregation_event()? {
            AggregationEvent::MouseInput { state, button, .. }
                if button == mouse_button && state == ElementState::Released =>
            {
                Some(self.pos().unwrap())
            }
            _ => None,
        }
    }

    pub fn detect_double_click(&self, button: MouseButton, max_distance: f64) -> Option<Point> {
        self.history
            .detect_double_click(button, Duration::from_millis(500), max_distance)
    }

    /// Detect a movement of >= `min_distance`. `min_distance` is in physical device coordinates
    /// while a mouse button was pressed.
    pub fn detect_movement(&self, button: MouseButton, min_distance: f64) -> Option<Movement> {
        self.history.detect_movement(button, min_distance)
    }

    /// Create a movement tracker based on this event.
    ///
    /// The event must be a mouse button event, otherwise `None`.
    pub fn track_movement(&self) -> Option<Movement> {
        let (sensor, _) = self.button_sensor_and_state()?;
        Some(Movement {
            sensor,
            began: self.time(),
            detected_after: Duration::ZERO,
            minimum_distance: 0.0,
            from: self.pos()?,
            delta: Vector::default(),
        })
    }

    /*
        pub fn detect_hold_and_movement(
            &self,
            button: MouseButton,
            min_hold: Duration,
            distance_considered_movement: scalar,
        ) -> Option<Movement> {
            detect::movement_after_hold(
                &self.history,
                button,
                min_hold,
                distance_considered_movement,
            )
        }
    */

    /// Detect several mouse gestures.
    ///
    /// `min_distance` specifies the minimum movement for the detection in physical device
    /// coordinates.
    pub fn detect_mouse_gesture(
        &self,
        button: MouseButton,
        min_distance: f64,
    ) -> Option<MouseGesture> {
        if let Some(point) = self.detect_double_click(button, min_distance) {
            return Some(MouseGesture::DoubleClick(point));
        }

        if let Some(point) = self.detect_click(button) {
            return Some(MouseGesture::Click(point));
        }

        if let Some(movement) = self.detect_movement(button, min_distance) {
            return Some(MouseGesture::Movement(movement));
        }

        if let Some(point) = self.detect_clicked(button) {
            return Some(MouseGesture::Clicked(point));
        }

        None
    }

    /// Returns the current duration since movement inactivity began.
    ///
    /// Returns `max_range` if the inactivity duration is equal or exceeds the `max_range`
    pub fn detect_movement_inactivity(
        &self,
        device: DeviceId,
        max_range: Duration,
        min_distance: f64,
    ) -> Option<Duration> {
        self.history
            .detect_movement_inactivity(device, max_range, min_distance)
        // TODO: This may return `UnitInterval` with respect to `max_range`?
    }

    pub fn time(&self) -> Instant {
        self.record().time()
    }

    fn record(&self) -> &EventRecord<E> {
        self.history.current().unwrap()
    }

    // Keyboard helpers.

    pub fn modifiers(&self) -> Modifiers {
        self.device_states().keyboard_modifiers()
    }

    pub fn keyboard_modifiers(&self) -> ModifiersState {
        self.modifiers().state()
    }

    /// The actual underlying event.
    ///
    /// Idea: What about implementing Deref for that?
    pub fn event(&self) -> &E {
        self.record().event()
    }

    pub(crate) fn to_aggregation_event(&self) -> Option<AggregationEvent> {
        self.record().event().to_aggregation_event()
    }

    pub fn pointing_device_state(&self, device: DeviceId) -> Option<&PointingDeviceState> {
        self.device_states().pointing_device(device)
    }

    pub fn device_states(&self) -> &DeviceStates {
        &self.record().states
    }
}
