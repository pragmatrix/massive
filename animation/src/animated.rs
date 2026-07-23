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
