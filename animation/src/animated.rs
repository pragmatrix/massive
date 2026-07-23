use std::time::Duration;

use crate::{AnimationCoordinator, BlendedAnimation, Interpolatable, Interpolation};

pub trait AnimationContext {
    fn animation_coordinator(&self) -> &AnimationCoordinator;
}

impl AnimationContext for AnimationCoordinator {
    fn animation_coordinator(&self) -> &AnimationCoordinator {
        self
    }
}

/// `Animated` represents an animated value over time.
///
/// `Animated` implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Animated<T: Send> {
    coordinator: AnimationCoordinator,
    /// The current value and the current state of the animation.
    inner: AnimatedRaw<T>,
}

impl<T: Interpolatable + Send> Animated<T> {
    pub(crate) fn new(coordinator: AnimationCoordinator, value: T) -> Self {
        Self {
            coordinator,
            inner: AnimatedRaw::new(value),
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
        self.inner
            .animate_if_changed(&self.coordinator, target_value, duration, interpolation);
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
    pub fn value(&mut self) -> &T {
        self.inner.value(&self.coordinator)
    }

    /// The latest stored value of this animated value without progressing active animations.
    pub fn latest(&self) -> &T {
        self.inner.latest()
    }

    /// The target / final value of this animated value after all current animations ran through or
    /// the current value one if no animations are active.
    pub fn target(&self) -> &T {
        self.inner.target()
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
pub struct AnimatedRaw<T>
where
    T: Send,
{
    /// The current value.
    value: T,
    /// The currently running animations.
    animation: BlendedAnimation<T>,
}

impl<T: Send + Interpolatable> From<T> for AnimatedRaw<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Send + Interpolatable> AnimatedRaw<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            animation: Default::default(),
        }
    }

    pub fn animate_if_changed(
        &mut self,
        context: &impl AnimationContext,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static + PartialEq,
    {
        if *self.target() == target_value {
            return;
        }

        self.animate(context, target_value, duration, interpolation);
    }

    pub fn animate(
        &mut self,
        context: &impl AnimationContext,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static,
    {
        let instant = context
            .animation_coordinator()
            .allocate_animation_time(duration);
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

    pub fn latest(&self) -> &T {
        &self.value
    }

    pub fn target(&self) -> &T {
        self.animation.target().unwrap_or(&self.value)
    }

    pub fn value(&mut self, context: &impl AnimationContext) -> &T {
        self.progress(context);
        self.latest()
    }

    fn progress(&mut self, context: &impl AnimationContext) {
        if self.animation.is_active() {
            let instant = context.animation_coordinator().current_cycle_time();
            if let Some(new_value) = self.animation.proceed(instant) {
                self.value = new_value;
            }
        }
    }

    pub fn is_animating(&self) -> bool {
        self.animation.is_active()
    }

    pub fn animation_count(&self) -> usize {
        self.animation.count()
    }
}
