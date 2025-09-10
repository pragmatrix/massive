use winit::event::{DeviceId, MouseButton};

/// A specific button on a device.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ButtonSensor {
    pub device: DeviceId,
    pub button: MouseButton,
}

impl ButtonSensor {
    pub fn new(device: DeviceId, button: MouseButton) -> Self {
        Self { device, button }
    }
}
