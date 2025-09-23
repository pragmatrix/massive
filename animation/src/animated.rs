use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{BlendedAnimation, Interpolatable, Interpolation, Tickery};

/// `Animated` represents an animated value over time.
///
/// `Animated` implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
#[derive(Debug)]
pub struct Animated<T: Send> {
    tickery: Arc<Tickery>,
    /// The current value and the current state of the animation.
    ///
    /// Mutex, because we want to access it through `&self` but modify it through the animator.
    inner: Mutex<AnimatedInner<T>>,
}

impl<T: Interpolatable + Send> Animated<T> {
    pub(crate) fn new(tickery: Arc<Tickery>, value: T) -> Self {
        Self {
            tickery,
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
    pub fn animate_to_if_changed(
        &mut self,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: 'static + PartialEq,
    {
        let mut inner = self.inner.lock().expect("poisoned");
        if *inner.final_value() == target_value {
            return;
        }
        let instant = self.tickery.animation_tick();
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
    pub fn animate_to(&mut self, target_value: T, duration: Duration, interpolation: Interpolation)
    where
        T: 'static,
    {
        let instant = self.tickery.animation_tick();

        let mut inner = self.inner.lock().expect("poisoned");
        let value = inner.value.clone();
        inner
            .animation
            .animate_to(value, instant, target_value, duration, interpolation);
    }

    /// Finalize all animations.
    ///
    /// This sets the current animated value to the final animation target value and stops all
    /// animations.
    ///
    /// Does nothing when no animation is active.
    pub fn finalize(&mut self) {
        let mut inner = self.inner.lock().expect("poisoned");
        if let Some(final_value) = inner.animation.commit() {
            inner.value = final_value
        }
    }

    /// The current value of this animated value.
    ///
    /// If an animation is active, this computes the current value from the animation and also
    /// subscribes for further ticks in the future.
    pub fn value(&self) -> T {
        let mut inner = self.inner.lock().expect("poisoned");
        if inner.animation.is_active() {
            // Detail: Don't retrieve the animation_tick if there is no animation active, because
            // this marks informs the update cycle tha an animation is active.
            //
            // Contract: **But** If the animation would return no value at the given instant, we
            // would not need to subscribe to further ticks. So there is always one more tick to
            // process, which may have unintended side effects and clients relying on that behavior.
            let instant = self.tickery.animation_tick();
            if let Some(new_value) = inner.animation.proceed(instant) {
                inner.value = new_value;
            }
        }

        inner.value.clone()
    }

    /// The final value of this animated value after all current animations ran through or the
    /// current one if no animations are active.
    pub fn final_value(&self) -> T {
        self.inner.lock().expect("poisoned").final_value().clone()
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
        self.inner.lock().expect("poisoned").animation.is_active()
    }

    /// Returns the number of active animation blendings.
    pub fn animation_count(&self) -> usize {
        self.inner.lock().expect("poisoned").animation.count()
    }
}

/// Shared by the animated value and the tickery.
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
