//! Movement with inactivity detection.

use std::time::Duration;

use derive_more::{Constructor, Deref};
use massive_geometry::{UnitInterval, Vector};
use winit::event::DeviceId;

use crate::Event;

use super::{Movement, movement};

#[derive(Clone, Debug, Deref)]
pub struct MovementInactivity {
    // TODO: may put this into a module named `inactivity` and name it `Movement`?
    /// When should the user be hinted about the inactivity.
    pub inactivity_hint_start: UnitInterval,
    /// After what duration should the inactivity begin.
    pub inactivity_duration: Duration,

    #[deref]
    movement: Movement,
    inactive: bool,
}

pub enum Result {
    Update { delta: Vector, inactive: bool },
    Commit { delta: Vector, inactive: bool },
    Cancel,
    Continue,
}

impl MovementInactivity {
    pub const DEFAULT_INACTIVITY_HINT_START: UnitInterval = UnitInterval::new_unchecked(0.5);
    pub const DEFAULT_INACTIVITY_DURATION: Duration = Duration::from_secs(2);

    pub fn new(movement: Movement) -> Self {
        Self {
            inactivity_hint_start: Self::DEFAULT_INACTIVITY_HINT_START,
            inactivity_duration: Self::DEFAULT_INACTIVITY_DURATION,
            inactive: false,
            movement,
        }
    }

    pub fn track(&mut self, event: &Event) -> Result {
        use movement::MovementChange::*;
        match self.movement.track(event) {
            Move(delta) => {
                let inactive = self.is_inactive(event);
                self.inactive = inactive;
                Result::Update { delta, inactive }
            }
            Commit(delta) => {
                let inactive = self.is_inactive(event);
                self.inactive = inactive;
                Result::Commit { delta, inactive }
            }
            Cancel => Result::Cancel,
            Continue => {
                let inactive = self.is_inactive(event);
                if inactive != self.inactive {
                    self.inactive = inactive;
                    return Result::Update {
                        delta: self.movement.delta,
                        inactive,
                    };
                }
                Result::Continue
            }
        }
    }

    /// `true` if the user is currently inactive.
    pub fn is_inactive(&self, event: &Event) -> bool {
        if let Some(duration) = event.detect_movement_inactivity(
            self.device(),
            self.inactivity_duration,
            self.minimum_distance,
        ) {
            return duration == self.inactivity_duration;
        }

        false
    }

    /// Return the current state of inactivity.
    pub fn inactivity_state(&self, event: &Event) -> InactivityState {
        if let Some(inactivity) = event.detect_movement_inactivity(
            self.device(),
            self.inactivity_duration,
            self.minimum_distance,
        ) {
            let hint_start = self.hint_duration_start();
            let inactive = inactivity == self.inactivity_duration;
            if inactivity < hint_start || inactive {
                return InactivityState::new(inactive, None);
            }

            let hint_at = inactivity - hint_start;
            let hint_duration = self.inactivity_duration - hint_start;
            let hint_at = UnitInterval::new(hint_at.as_secs_f64() / hint_duration.as_secs_f64());
            return InactivityState::new(false, Some(hint_at));
        }
        InactivityState::default()
    }

    fn device(&self) -> DeviceId {
        self.movement.sensor.device
    }

    /// The maximum duration the hint is shown. This can be used as the animation time.
    pub fn hint_duration(&self) -> Duration {
        self.inactivity_duration - self.hint_duration_start()
    }

    /// The duration after inactivity the hint should start showing up.
    fn hint_duration_start(&self) -> Duration {
        self.inactivity_duration
            .mul_f64(self.inactivity_hint_start.into())
    }
}

#[derive(Clone, Debug, Default, Constructor)]
pub struct InactivityState {
    /// User is currently confirmed inactive.
    pub inactive: bool,
    /// If set, there might be a period of confirmed coming up.
    ///
    /// This is a unit interval, where t should define the hinting progress, 0: we are at
    /// [`Self::inactivity_hint_start`] time. 1: inactivity_begins. `None`: Don't display an
    /// inactivity hint right now.
    pub show_hint: Option<UnitInterval>,
}

impl Movement {
    pub fn with_inactivity(self) -> MovementInactivity {
        MovementInactivity::new(self)
    }
}
