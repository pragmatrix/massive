use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::{Animator, BlendedAnimation, Ease, Interpolatable, Interpolation};

/// A timeline represents a value over time.
///
/// A timeline must be created to animate values.
#[derive(Debug)]
pub struct Timeline<T> {
    shared: Rc<RefCell<TimelineInner<T>>>,
}

impl<T: Interpolatable> Timeline<T> {
    pub(crate) fn new(value: T, instant: Instant) -> Self {
        let shared = Rc::new(RefCell::new(TimelineInner {
            value,
            pending_animations: Vec::new(),
            animations: Default::default(),
        }));

        let x = shared.clone() as Rc<dyn ReceivesTicks>;

        Self { shared }
    }

    pub fn animate_to(
        &mut self,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) {
        let mut shared = self.shared.borrow_mut();
        shared.pending_animations.push(PendingAnimation {
            to: target_value,
            duration,
            interpolation,
        });
    }
}

/// Shared by the timeline value and the animator.
#[derive(Debug)]
struct TimelineInner<T> {
    /// The current value.
    value: T,
    /// Pending animations.
    pending_animations: Vec<PendingAnimation<T>>,
    /// The currently running animations.
    animations: BlendedAnimation<T>,
}

impl<T: Interpolatable> TimelineInner<T> {
    pub fn tick(&mut self, instant: Instant) {
        // Integrate the pending animations.
        //
        // Even though the last animation added is the one the defines the ultimate ending time and
        // value, the ones before must be added too, so that their trajectory is blended into the
        // final animation.
        for pending in self.pending_animations.drain(..) {
            self.animations.animate_to(
                self.value.clone(),
                instant,
                pending.to,
                pending.duration,
                pending.interpolation,
            );
        }

        // Proceed with the blended animation an update the value.
        if let Some(value) = self.animations.proceed(instant) {
            self.value = value;
        }
    }
}

#[derive(Debug)]
struct PendingAnimation<T> {
    to: T,
    duration: Duration,
    interpolation: Interpolation,
}

trait ReceivesTicks {
    fn tick(&self, instant: Instant);
}

impl<T: Interpolatable> ReceivesTicks for RefCell<TimelineInner<T>> {
    fn tick(&self, instant: Instant) {
        let mut borrowed = self.borrow_mut();
        borrowed.tick(instant);
    }
}
