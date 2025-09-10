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

    pub fn left(&self) -> Option<ButtonSensor> {
        (self.button == MouseButton::Left).then_some(*self)
    }

    pub fn middle(&self) -> Option<ButtonSensor> {
        (self.button == MouseButton::Middle).then_some(*self)
    }

    pub fn right(&self) -> Option<ButtonSensor> {
        (self.button == MouseButton::Right).then_some(*self)
    }
}
