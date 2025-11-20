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

use winit::event::{DeviceId, MouseButton, WindowEvent};

pub trait DeviceIdExtensions {
    fn sensor(self, button: MouseButton) -> ButtonSensor;
}

impl DeviceIdExtensions for DeviceId {
    fn sensor(self, button: MouseButton) -> ButtonSensor {
        ButtonSensor::new(self, button)
    }
}
