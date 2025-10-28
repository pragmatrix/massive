//! Most of the code here was taken from the BS2 project.
mod event;
mod event_aggregator;
mod event_history;
mod event_history_detect;
mod event_manager;
mod external_event;
mod mouse_gesture;
mod progress;
mod sensor;
mod tracker;

pub use event::*;
pub use event_aggregator::*;
pub use event_manager::*;
pub use external_event::*;
pub use mouse_gesture::*;
pub use progress::*;
pub use sensor::*;
pub use tracker::*;

use winit::event::{DeviceId, MouseButton, Touch, WindowEvent};

pub trait WindowEventExtensions {
    fn device(&self) -> Option<DeviceId>;
}

impl WindowEventExtensions for WindowEvent {
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
            | WindowEvent::Touch(Touch { device_id, .. }) => Some(*device_id),
            _ => None,
        }
    }
}

pub trait DeviceIdExtensions {
    fn sensor(self, button: MouseButton) -> ButtonSensor;
}

impl DeviceIdExtensions for DeviceId {
    fn sensor(self, button: MouseButton) -> ButtonSensor {
        ButtonSensor::new(self, button)
    }
}
