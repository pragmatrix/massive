use std::time::Duration;

use crate::{BlendedAnimation, Interpolatable, Interpolation};
use crate::time::Instant;

pub trait AnimationContext {
    fn current_cycle_time(&mut self) -> Instant;

    fn allocate_animation_time(&mut self, duration: Duration) -> Instant;
}

#[derive(Debug)]
pub struct Animated<T>
where
    T: Send,
{
    /// The current value.
    value: T,
    /// The currently running animations.
    animation: BlendedAnimation<T>,
}

impl<T: Send + Interpolatable> From<T> for Animated<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Send + Interpolatable> Animated<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            animation: Default::default(),
        }
    }

    pub fn animate_if_changed(
        &mut self,
        context: &mut impl AnimationContext,
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
        context: &mut impl AnimationContext,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static,
    {
        let instant = context.allocate_animation_time(duration);
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

    pub fn value(&mut self, context: &mut impl AnimationContext) -> &T {
        self.progress(context);
        self.latest()
    }

    fn progress(&mut self, context: &mut impl AnimationContext) {
        if self.animation.is_active() {
            let instant = context.current_cycle_time();
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
