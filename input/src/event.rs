use std::{rc::Rc, time::Duration};

use massive_geometry::Point;
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

use crate::{MouseGesture, event_aggregator::DeviceStates, event_history::EventHistory};

use super::{ButtonSensor, WindowEventExtensions, event_history::EventRecord, tracker::Movement};

#[derive(Clone, Debug)]
pub struct Event {
    /// The event history, including the most recent event.
    history: Rc<EventHistory>,
}

impl Event {
    pub fn new(history: Rc<EventHistory>) -> Self {
        assert!(history.current().is_some());
        Self { history }
    }

    pub fn pressed(&self, sensor: ButtonSensor) -> bool {
        matches!(self.window_event(), Some(WindowEvent::MouseInput {
                device_id, state, ..
            }) if *device_id == sensor.device && *state == ElementState::Pressed)
    }

    pub fn released(&self, sensor: ButtonSensor) -> bool {
        matches!(self.window_event(), Some(WindowEvent::MouseInput {
                device_id, state, ..
            }) if *device_id == sensor.device && *state == ElementState::Released)
    }

    /// Returns the physical coordinates if the event is a pointer event.
    pub fn pos(&self) -> Option<Point> {
        self.pointing_device().and_then(|di| self.states().pos(di))
    }

    pub fn pointing_device(&self) -> Option<DeviceId> {
        self.window_event()?.pointing_device()
    }

    pub fn mouse_pressed(&self) -> Option<ButtonSensor> {
        match self.window_event()? {
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } if *state == ElementState::Pressed => Some(ButtonSensor::new(*device_id, *button)),
            _ => None,
        }
    }

    /// Returns the [`DeviceId`] of the event if this is a
    /// [`winit::event::WindowEvent::CursorMoved`] event.
    pub fn cursor_moved(&self) -> Option<DeviceId> {
        match self.window_event()? {
            WindowEvent::CursorMoved { device_id, .. } => Some(*device_id),
            _ => None,
        }
    }

    pub fn mouse_released(&self) -> Option<ButtonSensor> {
        match self.window_event()? {
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } if *state == ElementState::Released => Some(ButtonSensor::new(*device_id, *button)),
            _ => None,
        }
    }

    pub fn detect_click(&self, mouse_button: MouseButton) -> Option<Point> {
        match self.window_event()? {
            WindowEvent::MouseInput { state, button, .. }
                if *button == mouse_button && *state == ElementState::Pressed =>
            {
                Some(self.pos().unwrap())
            }
            _ => None,
        }
    }

    // This return the point where the mouse button was released, this way users can undo the click.
    pub fn detect_clicked(&self, mouse_button: MouseButton) -> Option<Point> {
        match self.window_event()? {
            WindowEvent::MouseInput { state, button, .. }
                if *button == mouse_button && *state == ElementState::Released =>
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

    // Detect a movement of >= `min_distance`. `min_distance` is in physical device coordinates.
    pub fn detect_movement(&self, button: MouseButton, min_distance: f64) -> Option<Movement> {
        self.history.detect_movement(button, min_distance)
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

    fn record(&self) -> &EventRecord {
        self.history.current().unwrap()
    }

    pub fn window_event(&self) -> Option<&WindowEvent> {
        self.record().window_event()
    }

    pub fn states(&self) -> &DeviceStates {
        &self.record().states
    }
}
