//! Most of the code here was taken from the BS2 project.
mod event_history_detect;
mod event;
mod event_aggregator;
mod event_history;
mod external_event;
mod mouse_gesture;
mod sensor;
mod tracker;

pub use event::*;
pub use event_aggregator::*;
pub use external_event::*;
pub use mouse_gesture::*;
pub use sensor::*;

use winit::event::{DeviceId, MouseButton, WindowEvent};

pub trait WindowEventExtensions {
    fn pointing_device(&self) -> Option<DeviceId>;
}

impl WindowEventExtensions for WindowEvent {
    fn pointing_device(&self) -> Option<DeviceId> {
        use winit::event::WindowEvent::*;
        match self {
            CursorMoved { device_id, .. }
            | CursorEntered { device_id }
            | CursorLeft { device_id }
            | MouseInput { device_id, .. } => Some(*device_id),
            _ => None,
        }
    }
}

pub trait DeviceIdExtensions {
    fn sensor(self, button: MouseButton) -> Sensor;
}

impl DeviceIdExtensions for DeviceId {
    fn sensor(self, button: MouseButton) -> Sensor {
        Sensor::new(self, button)
    }
}
