use std::time::Duration;

use crate::{time::Instant, Ease, Interpolatable, Interpolation};

#[derive(Debug)]
pub struct BlendedAnimation<T> {
    animations: Vec<Animation<T>>,
}

impl<T> Default for BlendedAnimation<T> {
    fn default() -> Self {
        Self {
            animations: Default::default(),
        }
    }
}

impl<T> BlendedAnimation<T> {
    /// Adds an animation on top of the stack of animations to blend.
    ///
    /// This animation is initially set up at 0% from the current value and then reaches 100% at end
    /// time the target value.
    ///
    /// The new end time for the blended animation and its value is now the new targeted end state.
    ///
    /// Animations on the stack reaching beyond the end time of `current_time` + `duration` won't be
    /// animated to their final value or end time anymore but gradually fade out.
    pub fn animate_to(
        &mut self,
        current_value: T,
        current_time: Instant,
        to: T,
        duration: Duration,
        interpolation: Interpolation,
    ) {
        self.animations.push(Animation {
            from: current_value,
            to,
            start_time: current_time,
            duration,
            interpolation,
        });
    }

    /// Any animation active?
    pub fn is_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// The final value if all animations ran through, or `None` if animations are not active.
    pub fn final_value(&self) -> Option<&T> {
        self.animations.last().map(|a| &a.to)
    }

    /// Proceed with the animation.
    ///
    /// Returns a computed current value at the instant, or None if there is no animation active.
    pub fn proceed(&mut self, instant: Instant) -> Option<T>
    where
        T: Interpolatable,
    {
        if self.animations.is_empty() {
            return None;
        }

        // The initial blended value is essentially ignored, but we need some value here and T does
        // not implement Default.
        let mut blended = self.animations[0].from.clone();
        let mut first_contributing_animation_index = 0;

        for (index, animation) in self.animations.iter().enumerate() {
            let t = animation.t_at(instant);
            // t might be larger than 1, if the animation does not end. But it's never negative.
            assert!(t >= 0.0);
            let value = animation.value_at_t(t);

            // The weight of the current animation relative to all previous ones.
            //
            // Weight is 1 at index 0, because a single animation does not need a blending factor,
            // otherwise the weight is the same t the animation uses to interpolate its value.
            let blend_weight = if index == 0 { 1. } else { t.min(1.0) };

            // Blend the current value (linearly) into the animations value and use it as the basis
            // for the next round.
            blended = Interpolatable::interpolate(&blended, &value, blend_weight);

            // The previous animations can be removed if the weight of the current animation is >=
            // 1.. They don't contribute to the final value anymore.
            if blend_weight >= 1. {
                first_contributing_animation_index = index;
            }
        }

        // Remove all animations until the first contributing one.
        self.animations.drain(0..first_contributing_animation_index);

        // If there is only one animation left, then it's the only contributing one, so if it's t is
        // already 1 (and therefore its relative weight must be 1, too), its final value is returned
        // now and the animation can be removed.
        if self.animations.len() == 1 && self.animations[0].t_at(instant) >= 1. {
            self.animations.clear();
        };

        Some(blended)
    }

    /// Remove all animations and return the final value.
    pub fn end(&mut self) -> Option<T> {
        if let Some(last) = self.animations.pop() {
            self.animations.clear();
            return Some(last.to);
        }
        None
    }

    /// Returns the number of blended animations.
    ///
    /// Useful for debugging purposes.
    pub fn count(&self) -> usize {
        self.animations.len()
    }
}

#[derive(Debug)]
struct Animation<T> {
    /// The value at the time the animation started.
    from: T,
    /// The target value at the end of the animation.
    to: T,
    /// The time the animation got started.
    start_time: Instant,
    /// The duration of the animation.
    duration: Duration,
    /// How to adjust t before interpolating the value.
    interpolation: Interpolation,
}

impl<T> Animation<T> {
    /// Compute the (linear) t value of this animation at the time `instant` ranging from 0.0 to
    /// 1.0, where 0.0 is < start_time and 1.0 >= end_time.
    pub fn t_at(&self, instant: Instant) -> f64 {
        if instant < self.start_time {
            return 0.;
        }

        let end_time = self.start_time + self.duration;
        if instant >= end_time {
            return 1.;
        }

        let t = (instant - self.start_time).as_secs_f64() / self.duration.as_secs_f64();

        // `t` may be NaN if duration is zero.
        if t >= 1.0 || !t.is_finite() {
            return 1.;
        }

        debug_assert!(t >= 0.);
        t
    }

    /// Compute the value at the given t including easing.
    ///
    /// If the t is < 0, the starting value is returned.
    /// If the t is >= 1, the ending value is returned.
    pub fn value_at_t(&self, t: f64) -> T
    where
        T: Interpolatable,
    {
        if t < 0. {
            return self.from.clone();
        }
        if t >= 1. {
            return self.to.clone();
        }

        // Apply the easing function to t.
        let t = Ease::interpolate(t, self.interpolation);

        Interpolatable::interpolate(&self.from, &self.to, t)
    }
}
