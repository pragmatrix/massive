use std::fmt;
use std::time::{Duration, Instant};

use interpolation::Interpolation;

use crate::{interpolation, Ease};

pub struct Animator {
    /// Animations that will start in the next tick.
    starting_animations: Vec<Box<dyn Animation>>,

    /// Animations currently active.
    active_animations: Vec<ActiveAnimation>,
}

impl fmt::Debug for Animator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Animator")
            .field("starting_animations", &self.starting_animations.len())
            .field("active_animations", &self.active_animations)
            .finish()
    }
}

struct ActiveAnimation {
    start_time: Instant,
    animation: Box<dyn Animation>,
}

impl fmt::Debug for ActiveAnimation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveAnimation")
            .field("start_time", &self.start_time)
            .finish()
    }
}

impl Animator {}

pub mod interpolate {
    use super::InterpolateFrom;

    pub fn from<V>(from: V) -> InterpolateFrom<V> {
        InterpolateFrom { from }
    }
}

struct InterpolateFrom<T> {
    from: T,
}

impl<V> InterpolateFrom<V> {
    pub fn to(self, to: V, duration: Duration) -> InterpolateFromTo<V> {
        InterpolateFromTo {
            from: self.from,
            to,
            duration,
            interpolation: Interpolation::default(),
        }
    }
}

#[derive(Debug)]
struct InterpolateFromTo<V> {
    from: V,
    to: V,
    duration: Duration,
    interpolation: Interpolation,
}

impl<V> InterpolateFromTo<V> {
    pub fn with(self, interpolation: Interpolation) -> Self {
        Self {
            interpolation,
            ..self
        }
    }

    pub fn apply<F: Fn(V) + 'static>(self, f: F) -> InterpolationApplication<V, F> {
        InterpolationApplication {
            interpolation: self,
            apply: f,
        }
    }
}

struct InterpolationApplication<V, F> {
    interpolation: InterpolateFromTo<V>,
    apply: F,
}

impl<V: Interpolatable + fmt::Debug, F: Fn(V)> InterpolationApplication<V, F> {
    pub fn start(self, animator: &mut Animator)
    where
        F: 'static,
        V: 'static,
    {
        animator.starting_animations.push(Box::new(self))
    }
}

impl<V: Interpolatable, F: Fn(V)> Animation for InterpolationApplication<V, F> {
    fn animate(&self, start_time: Instant, now: Instant) -> AnimationResult {
        debug_assert!(now >= start_time);
        let t = (now - start_time).as_secs_f64() / self.interpolation.duration.as_secs_f64();
        // `t` may be NaN if duration is zero.
        if t >= 1.0 || !t.is_finite() {
            (self.apply)(self.interpolation.to.clone());
            return AnimationResult::Done;
        }
        // Apply the (easing) function.
        let t = Ease::interpolate(t, self.interpolation.interpolation);
        let value =
            Interpolatable::interpolate(&self.interpolation.from, &self.interpolation.to, t);
        (self.apply)(value);
        AnimationResult::Continue
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum AnimationResult {
    /// Keep the animation running.
    Continue,
    // Remove it, it's done.
    Done,
}

trait Animation {
    fn animate(&self, start_time: Instant, now: Instant) -> AnimationResult;
}

/// For now we have to support `Clone`.
///
/// Other options: We pass 1.0 here and expect Self to return a clone for `to`, but can then never
/// be sure that it's exactly == `to`.`
pub trait Interpolatable: Clone {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self;
}

impl Interpolatable for f32 {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        (to - from) * (t as f32) + from
    }
}

impl Interpolatable for f64 {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        (to - from) * t + from
    }
}

impl Interpolatable for Instant {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        if to >= from {
            return *from + to.duration_since(*from).mul_f64(t);
        }
        *to + from.duration_since(*to).mul_f64(t)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };

    use super::{interpolate, Animation, AnimationResult};

    #[test]
    pub fn zero_duration_sets_final_value() {
        let value = Rc::new(RefCell::new(0.));
        let v2 = value.clone();

        let interpolation = interpolate::from(0.)
            .to(10., Duration::ZERO)
            .apply(move |v| *v2.borrow_mut() = v);

        let time = Instant::now();

        assert_eq!(interpolation.animate(time, time), AnimationResult::Done);
        assert_eq!(*value.borrow(), 10.);
    }
}
