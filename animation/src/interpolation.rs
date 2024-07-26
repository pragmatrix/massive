//! A module contains implementation of ease functions.
//! Adapted from: <https://github.com/pistondevelopers/interpolation> version 0.3.0

#[allow(missing_docs)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum Interpolation {
    #[default]
    Linear,

    QuadraticIn,
    QuadraticOut,
    QuadraticInOut,

    CubicIn,
    CubicOut,
    CubicInOut,

    QuarticIn,
    QuarticOut,
    QuarticInOut,

    QuinticIn,
    QuinticOut,
    QuinticInOut,

    SineIn,
    SineOut,
    SineInOut,

    CircularIn,
    CircularOut,
    CircularInOut,

    ExponentialIn,
    ExponentialOut,
    ExponentialInOut,

    ElasticIn,
    ElasticOut,
    ElasticInOut,

    BackIn,
    BackOut,
    BackInOut,

    BounceIn,
    BounceOut,
    BounceInOut,
}

#[allow(missing_docs)]
pub trait Ease {
    /// Calculate the eased value, normalized
    fn interpolate(self, f: Interpolation) -> Self;

    fn quadratic_in(self) -> Self;
    fn quadratic_out(self) -> Self;
    fn quadratic_in_out(self) -> Self;

    fn cubic_in(self) -> Self;
    fn cubic_out(self) -> Self;
    fn cubic_in_out(self) -> Self;

    fn quartic_in(self) -> Self;
    fn quartic_out(self) -> Self;
    fn quartic_in_out(self) -> Self;

    fn quintic_in(self) -> Self;
    fn quintic_out(self) -> Self;
    fn quintic_in_out(self) -> Self;

    fn sine_in(self) -> Self;
    fn sine_out(self) -> Self;
    fn sine_in_out(self) -> Self;

    fn circular_in(self) -> Self;
    fn circular_out(self) -> Self;
    fn circular_in_out(self) -> Self;

    fn exponential_in(self) -> Self;
    fn exponential_out(self) -> Self;
    fn exponential_in_out(self) -> Self;

    fn elastic_in(self) -> Self;
    fn elastic_out(self) -> Self;
    fn elastic_in_out(self) -> Self;

    fn back_in(self) -> Self;
    fn back_out(self) -> Self;
    fn back_in_out(self) -> Self;

    fn bounce_in(self) -> Self;
    fn bounce_out(self) -> Self;
    fn bounce_in_out(self) -> Self;
}

macro_rules! impl_ease_trait_for {
    ($T: ident) => {
        mod $T {
            #[allow(clippy::excessive_precision)]
            pub const PI_2: $T = 6.28318530717958647692528676655900576;

            pub fn clamp(p: $T) -> $T {
                match () {
                    _ if p > 1.0 => 1.0,
                    _ if p < 0.0 => 0.0,
                    _ => p,
                }
            }
        }
        impl Ease for $T {
            fn interpolate(self, f: Interpolation) -> Self {
                match f {
                    Interpolation::Linear => self,

                    Interpolation::QuadraticIn => self.quadratic_in(),
                    Interpolation::QuadraticOut => self.quadratic_out(),
                    Interpolation::QuadraticInOut => self.quadratic_in_out(),

                    Interpolation::CubicIn => self.cubic_in(),
                    Interpolation::CubicOut => self.cubic_out(),
                    Interpolation::CubicInOut => self.cubic_in_out(),

                    Interpolation::QuarticIn => self.quartic_in(),
                    Interpolation::QuarticOut => self.quartic_out(),
                    Interpolation::QuarticInOut => self.quartic_in_out(),

                    Interpolation::QuinticIn => self.quintic_in(),
                    Interpolation::QuinticOut => self.quintic_out(),
                    Interpolation::QuinticInOut => self.quintic_in_out(),

                    Interpolation::SineIn => self.sine_in(),
                    Interpolation::SineOut => self.sine_out(),
                    Interpolation::SineInOut => self.sine_in_out(),

                    Interpolation::CircularIn => self.circular_in(),
                    Interpolation::CircularOut => self.circular_out(),
                    Interpolation::CircularInOut => self.circular_in_out(),

                    Interpolation::ExponentialIn => self.exponential_in(),
                    Interpolation::ExponentialOut => self.exponential_out(),
                    Interpolation::ExponentialInOut => self.exponential_in_out(),

                    Interpolation::ElasticIn => self.elastic_in(),
                    Interpolation::ElasticOut => self.elastic_out(),
                    Interpolation::ElasticInOut => self.elastic_in_out(),

                    Interpolation::BackIn => self.back_in(),
                    Interpolation::BackOut => self.back_out(),
                    Interpolation::BackInOut => self.back_in_out(),

                    Interpolation::BounceIn => self.bounce_in(),
                    Interpolation::BounceOut => self.bounce_out(),
                    Interpolation::BounceInOut => self.bounce_in_out(),
                }
            }

            fn quadratic_in(self) -> Self {
                let p = $T::clamp(self);
                p * p
            }

            fn quadratic_out(self) -> Self {
                let p = $T::clamp(self);
                -(p * (p - 2.0))
            }

            fn quadratic_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    2.0 * p * p
                } else {
                    (-2.0 * p * p) + (4.0 * p) - 1.0
                }
            }

            fn cubic_in(self) -> Self {
                let p = $T::clamp(self);
                p * p * p
            }

            fn cubic_out(self) -> Self {
                let p = $T::clamp(self);
                let f = p - 1.0;
                f * f * f + 1.0
            }

            fn cubic_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    let f = (2.0 * p) - 2.0;
                    0.5 * f * f * f + 1.0
                }
            }

            fn quartic_in(self) -> Self {
                let p = $T::clamp(self);
                p * p * p * p
            }

            fn quartic_out(self) -> Self {
                let p = $T::clamp(self);
                let f = p - 1.0;
                f * f * f * (1.0 - p) + 1.0
            }

            fn quartic_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    8.0 * p * p * p * p
                } else {
                    let f = p - 1.0;
                    -8.0 * f * f * f * f + 1.0
                }
            }

            fn quintic_in(self) -> Self {
                let p = $T::clamp(self);
                p * p * p * p * p
            }

            fn quintic_out(self) -> Self {
                let p = $T::clamp(self);
                let f = p - 1.0;
                f * f * f * f * f + 1.0
            }

            fn quintic_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    16.0 * p * p * p * p * p
                } else {
                    let f = (2.0 * p) - 2.0;
                    0.5 * f * f * f * f * f + 1.0
                }
            }

            fn sine_in(self) -> Self {
                use self::$T::PI_2;
                let p = $T::clamp(self);
                ((p - 1.0) * PI_2).sin() + 1.0
            }

            fn sine_out(self) -> Self {
                use self::$T::PI_2;
                let p = $T::clamp(self);
                (p * PI_2).sin()
            }

            fn sine_in_out(self) -> Self {
                use std::$T::consts::PI;
                let p = $T::clamp(self);
                0.5 * (1.0 - (p * PI).cos())
            }

            fn circular_in(self) -> Self {
                let p = $T::clamp(self);
                1.0 - (1.0 - (p * p)).sqrt()
            }

            fn circular_out(self) -> Self {
                let p = $T::clamp(self);
                ((2.0 - p) * p).sqrt()
            }

            fn circular_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    0.5 * (1.0 - (1.0 - 4.0 * (p * p)).sqrt())
                } else {
                    0.5 * ((-((2.0 * p) - 3.0) * ((2.0 * p) - 1.0)).sqrt() + 1.0)
                }
            }

            fn exponential_in(self) -> Self {
                if self <= 0.0 {
                    0.0
                } else {
                    (2.0 as $T).powf(10.0 * (self.min(1.0) - 1.0))
                }
            }

            fn exponential_out(self) -> Self {
                if self >= 1.0 {
                    1.0
                } else {
                    1.0 - (2.0 as $T).powf(-10.0 * self.max(0.0))
                }
            }

            fn exponential_in_out(self) -> Self {
                if self <= 0.0 {
                    return 0.0;
                }
                if self >= 1.0 {
                    return 1.0;
                }

                if self < 0.5 {
                    0.5 * (2.0 as $T).powf((20.0 * self) - 10.0)
                } else {
                    -0.5 * (2.0 as $T).powf((-20.0 * self) + 10.0) + 1.0
                }
            }

            fn elastic_in(self) -> Self {
                use self::$T::PI_2;
                let p = $T::clamp(self);
                (13.0 * PI_2 * p).sin() * (2.0 as $T).powf(10.0 * (p - 1.0))
            }

            fn elastic_out(self) -> Self {
                use self::$T::PI_2;
                let p = $T::clamp(self);
                (-13.0 * PI_2 * (p + 1.0)).sin() * (2.0 as $T).powf(-10.0 * p) + 1.0
            }

            fn elastic_in_out(self) -> Self {
                use self::$T::PI_2;
                let p = $T::clamp(self);
                if p < 0.5 {
                    0.5 * (13.0 * PI_2 * (2.0 * p)).sin()
                        * (2.0 as $T).powf(10.0 * ((2.0 * p) - 1.0))
                } else {
                    0.5 * ((-13.0 * PI_2 * ((2.0 * p - 1.0) + 1.0)).sin()
                        * (2.0 as $T).powf(-10.0 * (2.0 * p - 1.0))
                        + 2.0)
                }
            }

            fn back_in(self) -> Self {
                use std::$T::consts::PI;
                let p = $T::clamp(self);
                p * p * p - p * (p * PI).sin()
            }

            fn back_out(self) -> Self {
                use std::$T::consts::PI;
                let p = $T::clamp(self);
                let f = 1.0 - p;
                1.0 - (f * f * f - f * (f * PI).sin())
            }

            fn back_in_out(self) -> Self {
                use std::$T::consts::PI;
                let p = $T::clamp(self);
                if p < 0.5 {
                    let f = 2.0 * p;
                    0.5 * (f * f * f - f * (f * PI).sin())
                } else {
                    let f = 1.0 - (2.0 * p - 1.0);
                    0.5 * (1.0 - (f * f * f - f * (f * PI).sin())) + 0.5
                }
            }

            fn bounce_in(self) -> Self {
                let p = $T::clamp(self);
                1.0 - Ease::bounce_out(1.0 - p)
            }

            fn bounce_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 4.0 / 11.0 {
                    (121.0 * p * p) / 16.0
                } else if p < 8.0 / 11.0 {
                    (363.0 / 40.0 * p * p) - (99.0 / 10.0 * p) + 17.0 / 5.0
                } else if p < 9.0 / 10.0 {
                    (4356.0 / 361.0 * p * p) - (35442.0 / 1805.0 * p) + 16061.0 / 1805.0
                } else {
                    (54.0 / 5.0 * p * p) - (513.0 / 25.0 * p) + 268.0 / 25.0
                }
            }

            fn bounce_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p < 0.5 {
                    0.5 * Ease::bounce_in(p * 2.0)
                } else {
                    0.5 * Ease::bounce_out(p * 2.0 - 1.0) + 0.5
                }
            }
        }
    };
}

impl_ease_trait_for!(f32);
impl_ease_trait_for!(f64);
