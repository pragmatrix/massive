use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{BlendedAnimation, Interpolatable, Interpolation, Tickery};

/// A timeline represents a value over time that can be animated.
///
/// Timeline implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Timeline<T: Send> {
    tickery: Arc<Tickery>,
    /// The current value and the current state of the animation.
    ///
    /// Mutex, because we want to access it through `&self` but modify it through the animator.
    inner: Mutex<TimelineInner<T>>,
}

impl<T: Interpolatable + Send> Timeline<T> {
    pub(crate) fn new(tickery: Arc<Tickery>, value: T) -> Self {
        Self {
            tickery,
            inner: TimelineInner {
                value,
                animation: Default::default(),
            }
            .into(),
        }
    }

    /// Animate to a target value in the given duration.
    ///
    /// When multiple animations happen in the same time slice, they are blended together.
    ///
    /// Animation starts on the next time the value is queried. This function does not change the
    /// current value, if it is currently not animating.
    pub fn animate_to(&mut self, target_value: T, duration: Duration, interpolation: Interpolation)
    where
        T: 'static,
    {
        let instant = self.tickery.current_tick();

        let mut inner = self.inner.lock().expect("poisoned");
        let value = inner.value.clone();
        inner
            .animation
            .animate_to(value, instant, target_value, duration, interpolation);
    }

    pub fn value(&self) -> T
    where
        T: Clone,
    {
        let instant = self.tickery.current_tick();
        let mut inner = self.inner.lock().expect("poisoned");
        if let Some(new_value) = inner.animation.proceed(instant) {
            inner.value = new_value;
        }

        inner.value.clone()
    }

    pub fn is_animating(&self) -> bool {
        self.inner.lock().expect("poisoned").animation.is_active()
    }
}

/// Shared by the timeline value and the tickery.
#[derive(Debug)]
struct TimelineInner<T>
where
    T: Send,
{
    /// The current value.
    value: T,
    /// The currently running animations.
    animation: BlendedAnimation<T>,
}
