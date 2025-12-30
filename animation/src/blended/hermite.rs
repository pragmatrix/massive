use std::{
    ops::{Add, Mul},
    time::Duration,
};

use crate::{time::Instant, Interpolatable};

#[derive(Debug)]
pub struct Hermite<T> {
    animation: Option<Animation<T>>,
}

impl<T> Default for Hermite<T> {
    fn default() -> Self {
        Self { animation: None }
    }
}

impl<T> Hermite<T> {
    /// Any animation active?
    pub fn is_active(&self) -> bool {
        self.animation.is_some()
    }

    /// The final value if all animations ran through, or `None` if animations are not active.
    pub fn final_value(&self) -> Option<&T> {
        self.animation.as_ref().map(|a| &a.to)
    }

    pub fn count(&self) -> usize {
        if self.animation.is_some() {
            1
        } else {
            0
        }
    }

    pub fn end(&mut self) -> Option<T> {
        self.animation.take().map(|a| a.to)
    }
}

impl<T> Hermite<T>
where
    T: Interpolatable + Add<Output = T> + Mul<f64, Output = T>,
{
    /// Adds an animation with hermite interpolation that maintains velocity continuity.
    ///
    /// The new animation replaces any existing animation, computing the current velocity
    /// to ensure smooth transition. This creates velocity-continuous motion where each
    /// new animation takes over seamlessly from the previous one.
    pub fn animate_to(
        &mut self,
        current_value: T,
        current_time: Instant,
        to: T,
        duration: Duration,
    ) {
        let from_tangent = if let Some(ref animation) = self.animation {
            let t = animation.t_at(current_time);
            // If previous animation has ended, start fresh like CubicOut
            if t >= 1.0 {
                let delta = to.clone() + (current_value.clone() * -1.0);
                delta * 3.0
            } else {
                // Compute velocity from existing animation at current time
                let velocity = animation.velocity_at(current_time);

                // To maintain aggressiveness when chaining animations:
                // We blend the existing velocity with the fresh direction boost.
                // This prevents the animation from becoming too sluggish as it decelerates,
                // while avoiding excessive overshoot.
                let velocity_tangent = velocity * duration.as_secs_f64();
                let delta = to.clone() + (current_value.clone() * -1.0);
                let direction_tangent = delta * 3.0;

                // Blend 60% existing velocity + 40% fresh direction
                // This maintains momentum while adapting to the new target
                velocity_tangent * 0.6 + direction_tangent * 0.4
            }
        } else {
            // No existing animation, start with CubicOut-like initial velocity
            // CubicOut has derivative of 3 at t=0, so initial tangent = 3 * (to - from)
            let delta = to.clone() + (current_value.clone() * -1.0);
            delta * 3.0
        };

        // End with zero velocity (decelerates to stop)
        let to_tangent = to.clone() * 0.0;

        self.animation = Some(Animation {
            from: current_value,
            to,
            from_tangent,
            to_tangent,
            start_time: current_time,
            duration,
        });
    }

    /// Proceed with the animation.
    ///
    /// Returns a computed current value at the instant, or None if there is no animation active.
    pub fn proceed(&mut self, instant: Instant) -> Option<T> {
        let animation = self.animation.as_ref()?;
        let t = animation.t_at(instant);

        // If animation is complete, clear it
        if t >= 1.0 {
            let final_value = animation.to.clone();
            self.animation = None;
            return Some(final_value);
        }

        Some(animation.value_at_t(t))
    }
}

#[derive(Debug)]
struct Animation<T> {
    from: T,
    to: T,
    from_tangent: T,
    to_tangent: T,
    start_time: Instant,
    duration: Duration,
}

impl<T> Animation<T>
where
    T: Add<Output = T> + Mul<f64, Output = T> + Clone,
{
    fn t_at(&self, instant: Instant) -> f64 {
        if instant < self.start_time {
            return 0.0;
        }

        let end_time = self.start_time + self.duration;
        if instant >= end_time {
            return 1.0;
        }

        let t = (instant - self.start_time).as_secs_f64() / self.duration.as_secs_f64();

        if t >= 1.0 || !t.is_finite() {
            return 1.0;
        }

        debug_assert!(t >= 0.0);
        t
    }

    fn value_at_t(&self, t: f64) -> T {
        if t <= 0.0 {
            return self.from.clone();
        }
        if t >= 1.0 {
            return self.to.clone();
        }

        hermite_interpolate(
            &self.from,
            &self.to,
            &self.from_tangent,
            &self.to_tangent,
            t,
        )
    }

    /// Compute the velocity (derivative) at a given instant.
    ///
    /// Returns velocity in units per second.
    fn velocity_at(&self, instant: Instant) -> T {
        let t = self.t_at(instant);

        // If before animation, velocity is zero
        if t < 0.0 {
            return self.from.clone() * 0.0;
        }
        // If after animation, velocity is zero
        if t > 1.0 {
            return self.from.clone() * 0.0;
        }

        // Derivative of hermite interpolation:
        // h'(t) = (6t²-6t)·p₀ + (3t²-4t+1)·m₀ + (-6t²+6t)·p₁ + (3t²-2t)·m₁
        let t2 = t * t;

        let dh00 = 6.0 * t2 - 6.0 * t;
        let dh10 = 3.0 * t2 - 4.0 * t + 1.0;
        let dh01 = -6.0 * t2 + 6.0 * t;
        let dh11 = 3.0 * t2 - 2.0 * t;

        let term0 = self.from.clone() * dh00;
        let term1 = self.from_tangent.clone() * dh10;
        let term2 = self.to.clone() * dh01;
        let term3 = self.to_tangent.clone() * dh11;

        let derivative = term0 + term1 + term2 + term3;

        // Convert from derivative with respect to normalized t to velocity (units/second)
        derivative * (1.0 / self.duration.as_secs_f64())
    }
}

/// Cubic hermite interpolation using only Add and Mul<f64> operations.
///
/// Formula: h(t) = (2t³-3t²+1)·p₀ + (t³-2t²+t)·m₀ + (-2t³+3t²)·p₁ + (t³-t²)·m₁
fn hermite_interpolate<T>(p0: &T, p1: &T, m0: &T, m1: &T, t: f64) -> T
where
    T: Add<Output = T> + Mul<f64, Output = T> + Clone,
{
    let t2 = t * t;
    let t3 = t2 * t;

    // Hermite basis functions
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;

    let term0 = p0.clone() * h00;
    let term1 = m0.clone() * h10;
    let term2 = p1.clone() * h01;
    let term3 = m1.clone() * h11;

    term0 + term1 + term2 + term3
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cubic_out(t: f64) -> f64 {
        let t = 1.0 - t;
        1.0 - t * t * t
    }

    #[test]
    fn hermite_matches_cubic_out_when_not_interrupted() {
        let mut hermite = Hermite::default();
        let start_time = Instant::now();
        let duration = Duration::from_secs(1);

        // Animate from 0.0 to 100.0
        hermite.animate_to(0.0, start_time, 100.0, duration);

        // Sample at various points and compare to CubicOut
        let test_points = [0.0, 0.1, 0.2, 0.3, 0.5, 0.7, 0.9, 1.0];

        for &t in &test_points {
            let instant = start_time + Duration::from_secs_f64(t * duration.as_secs_f64());
            let hermite_value = hermite.proceed(instant).unwrap();
            let expected = cubic_out(t) * 100.0;

            let diff = (hermite_value - expected).abs();
            assert!(
                diff < 1e-10,
                "At t={}: hermite={}, cubic_out={}, diff={}",
                t,
                hermite_value,
                expected,
                diff
            );
        }
    }

    #[test]
    fn hermite_velocity_matches_cubic_out_derivative() {
        let mut hermite = Hermite::default();
        let start_time = Instant::now();
        let duration = Duration::from_secs(1);

        hermite.animate_to(0.0, start_time, 100.0, duration);

        let animation = hermite.animation.as_ref().unwrap();

        // CubicOut derivative: d/dt[1-(1-t)³] = 3(1-t)²
        // At t=0: velocity = 3 * 1² = 3, scaled by (to-from) = 300
        // At t=1: velocity = 3 * 0² = 0
        let test_points = [
            (0.0, 300.0),
            (0.2, 192.0),
            (0.5, 75.0),
            (0.8, 12.0),
            (1.0, 0.0),
        ];

        for &(t, expected_velocity) in &test_points {
            let instant = start_time + Duration::from_secs_f64(t);
            let velocity = animation.velocity_at(instant);

            let diff = (velocity - expected_velocity).abs();
            assert!(
                diff < 1e-9,
                "At t={}: velocity={}, expected={}, diff={}",
                t,
                velocity,
                expected_velocity,
                diff
            );
        }
    }

    #[test]
    fn hermite_restarts_with_cubic_out_if_previous_ended() {
        let mut hermite = Hermite::default();
        let start_time = Instant::now();
        let duration = Duration::from_secs(1);

        // First animation from 0.0 to 100.0
        hermite.animate_to(0.0, start_time, 100.0, duration);

        // Time passes beyond the end of the first animation
        let after_end = start_time + duration + Duration::from_millis(100);

        // Start a new animation without calling proceed (so the first animation is still stored)
        // This should start fresh with CubicOut velocity, not try to continue from zero velocity
        hermite.animate_to(100.0, after_end, 200.0, duration);

        // Check that velocity at the start of second animation matches CubicOut
        let animation = hermite.animation.as_ref().unwrap();
        let velocity = animation.velocity_at(after_end);
        let expected_velocity = 3.0 * (200.0 - 100.0); // CubicOut starting velocity

        let diff = (velocity - expected_velocity).abs();
        assert!(
            diff < 1e-9,
            "After ended animation, new animation should start with CubicOut velocity. Got {}, expected {}",
            velocity, expected_velocity
        );
    }

    #[test]
    fn hermite_maintains_aggressiveness_in_rapid_succession() {
        let mut hermite = Hermite::default();
        let start_time = Instant::now();
        let duration = Duration::from_millis(300);

        // Simulate rapid succession: animate to 100, then 200, then 300
        // Each new animation starts 50ms after the previous one
        hermite.animate_to(0.0, start_time, 100.0, duration);

        let t1 = start_time + Duration::from_millis(50);
        let val1 = hermite.proceed(t1).unwrap();
        hermite.animate_to(val1, t1, 200.0, duration);

        let t2 = t1 + Duration::from_millis(50);
        let val2 = hermite.proceed(t2).unwrap();
        hermite.animate_to(val2, t2, 300.0, duration);

        // Check that each new animation has significant initial velocity
        let animation = hermite.animation.as_ref().unwrap();
        let velocity_at_start = animation.velocity_at(t2);

        println!("Value at t1: {}", val1);
        println!("Value at t2: {}", val2);
        println!("Velocity at start of 3rd animation: {}", velocity_at_start);

        // The velocity should be aggressive - at least as much as a fresh CubicOut would be
        // A fresh CubicOut from val2 to 300.0 would have velocity = 3 * (300 - val2)
        let fresh_cubic_velocity = 3.0 * (300.0 - val2);
        println!("Fresh CubicOut velocity would be: {}", fresh_cubic_velocity);

        // With the boost, velocity should be greater than just the decaying velocity
        assert!(
            velocity_at_start > fresh_cubic_velocity * 0.8,
            "Rapid succession should maintain aggressive velocity. Got {}, expected at least {}",
            velocity_at_start,
            fresh_cubic_velocity * 0.8
        );
    }
}
