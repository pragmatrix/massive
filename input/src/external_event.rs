use super::WindowEvent;
use std::time::Instant;
use winit::window::WindowId;

#[derive(Debug)]
pub enum ExternalEvent {
    Window {
        window: WindowId,
        event: WindowEvent,
        time: Instant,
    },
}

impl ExternalEvent {
    pub fn time(&self) -> Instant {
        match *self {
            Self::Window { time, .. } => time,
        }
    }
}
