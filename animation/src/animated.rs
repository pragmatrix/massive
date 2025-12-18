use std::time::Duration;

use parking_lot::Mutex;

use crate::{AnimationCoordinator, BlendedAnimation, Interpolatable, Interpolation};

/// `Animated` represents an animated value over time.
///
/// `Animated` implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Animated<T: Send> {
    coordinator: AnimationCoordinator,
    /// The current value and the current state of the animation.
    ///
    /// Mutex, because we want to access it through `&self` but modify it through the animator.
    inner: Mutex<AnimatedInner<T>>,
}

impl<T: Interpolatable + Send> Animated<T> {
    pub(crate) fn new(coordinator: AnimationCoordinator, value: T) -> Self {
        Self {
            coordinator,
            inner: AnimatedInner {
                value,
                animation: Default::default(),
            }
            .into(),
        }
    }

    /// Animate to a target value if its different from the current target value.
    ///
    /// Ergonomics: This should probably be the default behavior.
    pub fn animate_if_changed(
        &mut self,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static + PartialEq,
    {
        let mut inner = self.inner.lock();
        if *inner.final_value() == target_value {
            return;
        }
        let instant = self.coordinator.allocate_animation_time(duration);
        let value = inner.value.clone();
        inner
            .animation
            .animate_to(value, instant, target_value, duration, interpolation);
    }

    /// Animate to a target value in the given duration.
    ///
    /// When multiple animations happen in the same time slice, they are blended together.
    ///
    /// Animation starts on the next time the value is queried. This function does not change the
    /// current value, if it is currently not animating.
    pub fn animate(&mut self, target_value: T, duration: Duration, interpolation: Interpolation)
    where
        T: 'static,
    {
        let instant = self.coordinator.allocate_animation_time(duration);

        let mut inner = self.inner.lock();
        let value = inner.value.clone();
        inner
            .animation
            .animate_to(value, instant, target_value, duration, interpolation);
    }

    /// Stop all animations, and set the current value.
    pub fn set_immediately(&mut self, value: T) {
        let mut inner = self.inner.lock();
        inner.animation.end();
        inner.value = value;
    }

    /// Finish all animations.
    ///
    /// This sets the current animated value to the final animation target value and stops all
    /// animations.
    ///
    /// Does nothing when no animation is active.
    pub fn finish(&mut self) {
        let mut inner = self.inner.lock();
        if let Some(final_value) = inner.animation.end() {
            inner.value = final_value
        }
    }

    /// The current value of this animated value.
    ///
    /// If an animation is active, this computes the current value from the animation.
    pub fn value(&self) -> T {
        let mut inner = self.inner.lock();
        if inner.animation.is_active() {
            let instant = self.coordinator.current_cycle_time();
            if let Some(new_value) = inner.animation.proceed(instant) {
                inner.value = new_value;
            }
        }

        inner.value.clone()
    }

    /// The final value of this animated value after all current animations ran through or the
    /// current value one if no animations are active.
    pub fn final_value(&self) -> T {
        self.inner.lock().final_value().clone()
    }

    /// `true` if this is currently animating.
    ///
    /// Detail: Even if this is returning `false`, the client needing the value in response to
    /// ApplyAnimations may not have seen the final value yet. For example, this happens when the
    /// current value is retrieved while not in response to an ApplyAnimations event.
    ///
    /// So if this returns `false`, the value should be used to apply to the animated values that
    /// need updates.
    ///
    /// Ergonomics: Foolproof!
    pub fn is_animating(&self) -> bool {
        self.inner.lock().animation.is_active()
    }

    /// Returns the number of active animation blendings.
    pub fn animation_count(&self) -> usize {
        self.inner.lock().animation.count()
    }
}

#[derive(Debug)]
struct AnimatedInner<T>
where
    T: Send,
{
    /// The current value.
    value: T,
    /// The currently running animations.
    animation: BlendedAnimation<T>,
}

impl<T: Send> AnimatedInner<T> {
    pub fn final_value(&self) -> &T {
        self.animation.final_value().unwrap_or(&self.value)
    }
}
