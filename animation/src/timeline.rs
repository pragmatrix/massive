use std::{
    cell::{Ref, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use crate::{
    time::Instant, BlendedAnimation, Interpolatable, Interpolation, ReceivesTicks, TickResponse,
    Tickery,
};

/// A timeline represents a value over time that can be animated.
///
/// Timeline implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Timeline<T> {
    tickery: Rc<Tickery>,
    shared: Rc<RefCell<TimelineInner<T>>>,
}

impl<T: Interpolatable> Timeline<T> {
    pub(crate) fn new(tickery: Rc<Tickery>, value: T) -> Self {
        let shared = Rc::new(RefCell::new(TimelineInner {
            value,
            scheduled: Vec::new(),
            animation: Default::default(),
        }));

        Self { tickery, shared }
    }

    pub fn animate_to(&mut self, target_value: T, duration: Duration, interpolation: Interpolation)
    where
        T: 'static,
    {
        let mut shared = self.shared.borrow_mut();
        let receiving_ticks = shared.is_animating();

        shared.scheduled.push(ScheduledAnimation {
            to: target_value,
            duration,
            interpolation,
        });

        if !receiving_ticks {
            let tick_receiver = Rc::downgrade(&self.shared) as Weak<dyn ReceivesTicks>;
            self.tickery.start_sending(tick_receiver)
        }
    }

    pub fn value(&self) -> T
    where
        T: Clone,
    {
        self.shared.borrow().value.clone()
    }

    pub fn value_ref(&self) -> Ref<T> {
        let r = self.shared.borrow();
        Ref::map(r, |i| &i.value)
    }

    pub fn is_animating(&self) -> bool {
        self.shared.borrow().is_animating()
    }
}

/// Shared by the timeline value and the tickery.
#[derive(Debug)]
struct TimelineInner<T> {
    /// The current value.
    value: T,
    /// Pending animations. The animations added in the next tick.
    scheduled: Vec<ScheduledAnimation<T>>,
    /// The currently running animations.
    animation: BlendedAnimation<T>,
}

impl<T: Interpolatable> TimelineInner<T> {
    pub fn is_animating(&self) -> bool {
        self.animation.is_active() || !self.scheduled.is_empty()
    }

    pub fn tick(&mut self, instant: Instant) -> TickResponse {
        // Integrate the pending animations.
        //
        // Even though the last animation added is the one the defines the ultimate ending time and
        // value, the ones before must be added too, so that their trajectory is blended into the
        // final animation.
        for pending in self.scheduled.drain(..) {
            self.animation.animate_to(
                self.value.clone(),
                instant,
                pending.to,
                pending.duration,
                pending.interpolation,
            );
        }

        // Proceed with the blended animation an update the value.
        if let Some(value) = self.animation.proceed(instant) {
            self.value = value;
        }

        if self.animation.is_active() {
            TickResponse::Continue
        } else {
            TickResponse::Stop
        }
    }
}

/// An amimation scheduled to start at the next tick.
#[derive(Debug)]
struct ScheduledAnimation<T> {
    to: T,
    duration: Duration,
    interpolation: Interpolation,
}

impl<T: Interpolatable> ReceivesTicks for RefCell<TimelineInner<T>> {
    fn tick(&self, instant: Instant) -> TickResponse {
        self.borrow_mut().tick(instant)
    }
}
