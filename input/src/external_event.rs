use std::{fmt, time::Instant};

use winit::{
    event::{self, DeviceId, ElementState, Modifiers, MouseButton},
    window::WindowId,
};

use super::WindowEvent;
use massive_geometry::Point;

#[derive(Debug)]
pub struct ExternalEvent<E: InputEvent> {
    pub scope: E::ScopeId,
    pub event: E,
    pub time: Instant,
}

impl ExternalEvent<WindowEvent> {
    pub fn from_window_event(window: WindowId, event: WindowEvent, time: Instant) -> Self {
        Self {
            scope: window,
            event,
            time,
        }
    }
}

impl<E: InputEvent> ExternalEvent<E> {
    pub fn time(&self) -> Instant {
        self.time
    }
}

pub trait InputEvent: fmt::Debug + 'static {
    type ScopeId: fmt::Debug;

    /// See [`AggregationEvent`].
    fn to_aggregation_event(&self) -> Option<AggregationEvent>;

    /// The device an event is related to.
    fn device(&self) -> Option<DeviceId>;
}

impl InputEvent for WindowEvent {
    type ScopeId = WindowId;

    fn to_aggregation_event(&self) -> Option<AggregationEvent> {
        match *self {
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => Some(AggregationEvent::CursorMoved {
                device_id,
                position: (position.x, position.y).into(),
            }),
            WindowEvent::CursorEntered { device_id } => {
                Some(AggregationEvent::CursorEntered { device_id })
            }
            WindowEvent::CursorLeft { device_id } => {
                Some(AggregationEvent::CursorLeft { device_id })
            }
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => Some(AggregationEvent::MouseInput {
                device_id,
                state,
                button,
            }),
            WindowEvent::ModifiersChanged(modifiers) => {
                Some(AggregationEvent::ModifiersChanged(modifiers))
            }
            _ => None,
        }
    }

    fn device(&self) -> Option<DeviceId> {
        match self {
            WindowEvent::KeyboardInput { device_id, .. }
            | WindowEvent::CursorMoved { device_id, .. }
            | WindowEvent::CursorEntered { device_id }
            | WindowEvent::CursorLeft { device_id }
            | WindowEvent::MouseWheel { device_id, .. }
            | WindowEvent::MouseInput { device_id, .. }
            | WindowEvent::PinchGesture { device_id, .. }
            | WindowEvent::PanGesture { device_id, .. }
            | WindowEvent::DoubleTapGesture { device_id, .. }
            | WindowEvent::RotationGesture { device_id, .. }
            | WindowEvent::TouchpadPressure { device_id, .. }
            | WindowEvent::AxisMotion { device_id, .. }
            | WindowEvent::Touch(event::Touch { device_id, .. }) => Some(*device_id),
            _ => None,
        }
    }
}

/// A distilled event representation to support state aggregation (i.e. tracking positions, button
/// states, keyboard modifiers).
pub enum AggregationEvent {
    CursorMoved {
        device_id: DeviceId,
        position: Point,
    },
    CursorEntered {
        device_id: DeviceId,
    },
    CursorLeft {
        device_id: DeviceId,
    },
    MouseInput {
        device_id: DeviceId,
        state: ElementState,
        button: MouseButton,
    },
    ModifiersChanged(Modifiers),
}
