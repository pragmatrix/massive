use std::time::{Duration, Instant};

use winit::event::{ElementState, WindowEvent};

use crate::{Event, ButtonSensor};
use massive_geometry::{Point, Vector};

// `Clone` because of the borrow checker.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Movement {
    /// What was moved?
    pub sensor: ButtonSensor,
    /// The instant when the movement began.
    pub began: Instant,
    /// Time it took to detect the movement.
    pub detected_after: Duration,
    /// The origin of the movement. The point from where the movement started.
    pub from: Point,
    /// The current movement vector relative to `from`.
    pub delta: Vector,
    /// What was the minimum distance used to detect this movement?
    pub minimum_distance: f64,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Result {
    /// A movement to another location was detected. [`Vector`] describes the distance from the
    /// origin hit position (`from`) to the current position.  
    /// This is _always_ the same as `movement` and provided for convenience.
    Move(Vector),
    /// The movement was completed successfully and the new final position should be used.
    /// [`Vector`] describes the distance from the original hit position to the current position.  
    /// This is _always_ the same as `movement` and provided for convenience.
    ///
    /// Note that the event system must guarantee that the final [`Result::Commit`] is equal to
    /// the most recent vector sent by [`Result::Move`].  
    Commit(Vector),
    /// Cancelled by another event that has the power to cancel a movement gesture.
    Cancel,
    /// Event was unrelated to movements, movement stays active.
    Continue,
}

impl Movement {
    /// Tracks movements. Updates `movement` if current position changed.
    pub fn track(&mut self, event: &Event) -> Result {
        if self.cancels(event) {
            self.delta = Vector::default();
            return Result::Cancel;
        }

        if event.pointing_device() != Some(self.sensor.device) {
            return Result::Continue;
        }

        let movement = event.pos().unwrap() - self.from;

        if event.released(self.sensor) {
            self.delta = movement;
            return Result::Commit(movement);
        }

        if movement != self.delta {
            self.delta = movement;
            return Result::Move(movement);
        }

        Result::Continue
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
