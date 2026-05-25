use std::time::Duration;

use crate::{AnimationCoordinator, BlendedAnimation, Interpolatable, Interpolation};

/// `Animated` represents an animated value over time.
///
/// `Animated` implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Animated<T: Send> {
    coordinator: AnimationCoordinator,
    /// The current value and the current state of the animation.
    inner: AnimatedInner<T>,
}

impl<T: Interpolatable + Send> Animated<T> {
    pub(crate) fn new(coordinator: AnimationCoordinator, value: T) -> Self {
        Self {
            coordinator,
            inner: AnimatedInner::new(value),
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
        self.inner.animate_if_changed(
            &self.coordinator,
            target_value,
            duration,
            interpolation,
        );
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
        self.inner
            .animate(&self.coordinator, target_value, duration, interpolation);
    }

    /// Stop all animations, and set the current value.
    pub fn set_immediately(&mut self, value: T) {
        self.inner.set_immediately(value);
    }

    /// Finish all animations.
    ///
    /// This sets the current animated value to the final animation target value and stops all
    /// animations.
    ///
    /// Does nothing when no animation is active.
    pub fn finish(&mut self) {
        self.inner.finish();
    }

    /// The current value of this animated value, progressing active animations first.
    pub fn value(&mut self) -> T {
        self.inner.progressed_value(&self.coordinator)
    }

    /// The latest stored value of this animated value without progressing active animations.
    pub fn latest_value(&self) -> T {
        self.inner.value()
    }

    /// The final value of this animated value after all current animations ran through or the
    /// current value one if no animations are active.
    pub fn final_value(&self) -> T {
        self.inner.final_value().clone()
    }

    /// `true` if this is currently animating.
    ///
    /// Detail: Even if this is returning `false`, the client needing the value in response to
    /// `ApplyAnimations` may not have seen the final value yet. For example, this happens when the
    /// current value is retrieved while not in response to an `ApplyAnimations` event.
    ///
    /// So if this returns `false`, the value should be used to apply to the animated values that
    /// need updates.
    ///
    /// Ergonomics: Foolproof!
    pub fn is_animating(&self) -> bool {
        self.inner.is_animating()
    }

    /// Returns the number of active animation blendings.
    pub fn animation_count(&self) -> usize {
        self.inner.animation_count()
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

impl<T: Send + Interpolatable> AnimatedInner<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            animation: Default::default(),
        }
    }

    pub fn animate_if_changed(
        &mut self,
        coordinator: &AnimationCoordinator,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static + PartialEq,
    {
        if *self.final_value() == target_value {
            return;
        }

        self.animate(coordinator, target_value, duration, interpolation);
    }

    pub fn animate(
        &mut self,
        coordinator: &AnimationCoordinator,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static,
    {
        let instant = coordinator.allocate_animation_time(duration);
        let value = self.value.clone();
        self.animation
            .animate_to(value, instant, target_value, duration, interpolation);
    }

    pub fn set_immediately(&mut self, value: T) {
        self.animation.end();
        self.value = value;
    }

    pub fn finish(&mut self) {
        if let Some(final_value) = self.animation.end() {
            self.value = final_value;
        }
    }

    pub fn final_value(&self) -> &T {
        self.animation.final_value().unwrap_or(&self.value)
    }

    pub fn value(&self) -> T {
        self.value.clone()
    }

    pub fn progressed_value(&mut self, coordinator: &AnimationCoordinator) -> T {
        if self.animation.is_active() {
            let instant = coordinator.current_cycle_time();
            if let Some(new_value) = self.animation.proceed(instant) {
                self.value = new_value;
            }
        }
        self.value.clone()
    }

    pub fn is_animating(&self) -> bool {
        self.animation.is_active()
    }

    pub fn animation_count(&self) -> usize {
        self.animation.count()
    }
}
