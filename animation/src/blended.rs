use std::{
    ops::{Add, Mul},
    time::Duration,
};

use crate::{time::Instant, Interpolatable, Interpolation};

mod hermite;
mod linear;

pub use hermite::Hermite;
pub use linear::Linear;

#[derive(Debug)]
pub enum BlendedAnimation<T: Interpolatable> {
    Linear(Linear<T>),
    Hermite(Hermite<T>),
}

impl<T: Interpolatable> Default for BlendedAnimation<T> {
    fn default() -> Self {
        Self::Linear(Linear::default())
    }
}

impl<T: Interpolatable> BlendedAnimation<T> {
    pub fn animate_to(
        &mut self,
        current_value: T,
        current_time: Instant,
        to: T,
        duration: Duration,
        interpolation: Interpolation,
    ) where
        T: Add<Output = T> + Mul<f64, Output = T>,
    {
        match self {
            Self::Linear(inner) => {
                inner.animate_to(current_value, current_time, to, duration, interpolation)
            }
            Self::Hermite(inner) => {
                // Hermite ignores the interpolation parameter - it uses velocity continuity
                inner.animate_to(current_value, current_time, to, duration)
            }
        }
    }

    pub fn is_active(&self) -> bool {
        match self {
            Self::Linear(inner) => inner.is_active(),
            Self::Hermite(inner) => inner.is_active(),
        }
    }

    pub fn final_value(&self) -> Option<&T> {
        match self {
            Self::Linear(inner) => inner.final_value(),
            Self::Hermite(inner) => inner.final_value(),
        }
    }

    pub fn end(&mut self) -> Option<T> {
        match self {
            Self::Linear(inner) => inner.end(),
            Self::Hermite(inner) => inner.end(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Self::Linear(inner) => inner.count(),
            Self::Hermite(inner) => inner.count(),
        }
    }
}

impl<T> BlendedAnimation<T>
where
    T: Interpolatable + Add<Output = T> + Mul<f64, Output = T>,
{
    pub fn proceed(&mut self, instant: Instant) -> Option<T> {
        match self {
            Self::Linear(inner) => inner.proceed(instant),
            Self::Hermite(inner) => inner.proceed(instant),
        }
    }
}
