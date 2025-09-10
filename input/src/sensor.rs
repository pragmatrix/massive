use winit::event::{DeviceId, MouseButton};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Sensor {
    pub device: DeviceId,
    pub button: MouseButton,
}

impl Sensor {
    pub fn new(device: DeviceId, button: MouseButton) -> Self {
        Self { device, button }
    }

    pub fn left(&self) -> Option<Sensor> {
        (self.button == MouseButton::Left).then_some(*self)
    }

    pub fn middle(&self) -> Option<Sensor> {
        (self.button == MouseButton::Middle).then_some(*self)
    }

    pub fn right(&self) -> Option<Sensor> {
        (self.button == MouseButton::Right).then_some(*self)
    }
}
