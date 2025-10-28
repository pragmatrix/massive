use std::time::{Duration, Instant};

use log::error;
use winit::event::{ElementState, WindowEvent};

use crate::{ButtonSensor, Event, Progress};
use massive_geometry::{Point, Vector};

// `Clone` because of the borrow checker.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Movement {
    /// Which sensor triggered the move?
    pub sensor: ButtonSensor,
    /// The instant when the movement began.
    pub began: Instant,
    /// Time it took to detect the movement.
    pub detected_after: Duration,
    /// What was the minimum distance used to detect this movement?
    pub minimum_distance: f64,

    /// The origin of the movement. The point from where the movement started.
    pub from: Point,

    /// The current movement vector relative to `from`.
    pub delta: Vector,
}

impl Movement {
    pub fn track_delta(&mut self, event: &Event) -> Option<Progress<Vector>> {
        self.track(event).map(|p| p.map(|m| m.delta))
    }

    pub fn track_to(&mut self, event: &Event) -> Option<Progress<Point>> {
        self.track(event).map(|p| p.map(|m| m.to()))
    }

    /// Tracks movements. Updates `movement` if current position changed.
    ///
    /// `None` if the event was unrelated to the movement and it stays active.
    pub fn track(&mut self, event: &Event) -> Option<Progress<&Movement>> {
        if self.cancels(event) {
            self.delta = Vector::default();
            return Some(Progress::Cancel);
        }

        if event.device() != Some(self.sensor.device) {
            return None;
        }
        let pos = event.pos()?;
        let movement = pos - self.from;

        if event.released(self.sensor) {
            if self.delta != movement {
                error!(
                    "Internal error: movement is different from current delta when the sensor got released, did we miss movement updates?"
                )
            }
            return Some(Progress::Commit);
        }

        if movement != self.delta {
            self.delta = movement;
            return Some(Progress::Proceed(self));
        }

        None
    }

    /// Returns the current point of the movement.
    pub fn to(&self) -> Point {
        self.from + self.delta
    }

    fn cancels(&self, event: &Event) -> bool {
        // Cancellation of a movement that involves the mouse happens when _any_ mouse button is
        // pressed.
        matches!(
            event.window_event(),
            Some(WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            })
        )
    }
}
