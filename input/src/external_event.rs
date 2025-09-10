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
    FrameTick(Instant),
}

impl ExternalEvent {
    pub fn time(&self) -> Instant {
        use ExternalEvent::*;
        match *self {
            Window { time, .. } => time,
            FrameTick(time) => time,
        }
    }
}
