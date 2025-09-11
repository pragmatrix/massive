use std::time::Instant;

use winit::window::WindowId;

use super::WindowEvent;

#[derive(Debug)]
pub enum ExternalEvent {
    Window {
        window: WindowId,
        event: WindowEvent,
        time: Instant,
    },
}

impl ExternalEvent {
    pub fn from_window_event(window: WindowId, event: WindowEvent, time: Instant) -> Self {
        Self::Window {
            window,
            event,
            time,
        }
    }

    pub fn time(&self) -> Instant {
        match *self {
            Self::Window { time, .. } => time,
        }
    }
}
