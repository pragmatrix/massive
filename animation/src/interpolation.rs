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
                use std::$T::consts::FRAC_PI_2;
                let p = $T::clamp(self);
                1.0 - (p * FRAC_PI_2).cos()
            }

            fn sine_out(self) -> Self {
                use std::$T::consts::FRAC_PI_2;
                let p = $T::clamp(self);
                (p * FRAC_PI_2).sin()
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
                let p = $T::clamp(self);
                if p == 0.0 {
                    return 0.0;
                }
                if p == 1.0 {
                    return 1.0;
                }
                let c4: $T = ((2.0 as $T) * std::$T::consts::PI) / (3.0 as $T);
                -(2.0 as $T).powf(10.0 * p - 10.0) * ((p * 10.0 - 10.75) * c4).sin()
            }

            fn elastic_out(self) -> Self {
                let p = $T::clamp(self);
                if p == 0.0 {
                    return 0.0;
                }
                if p == 1.0 {
                    return 1.0;
                }
                let c4: $T = ((2.0 as $T) * std::$T::consts::PI) / (3.0 as $T);
                (2.0 as $T).powf(-10.0 * p) * ((p * 10.0 - 0.75) * c4).sin() + 1.0
            }

            fn elastic_in_out(self) -> Self {
                let p = $T::clamp(self);
                if p == 0.0 {
                    return 0.0;
                }
                if p == 1.0 {
                    return 1.0;
                }
                let c5: $T = ((2.0 as $T) * std::$T::consts::PI) / (4.5 as $T);
                if p < 0.5 {
                    -0.5 * (2.0 as $T).powf(20.0 * p - 10.0) * ((20.0 * p - 11.125) * c5).sin()
                } else {
                    0.5 * (2.0 as $T).powf(-20.0 * p + 10.0) * ((20.0 * p - 11.125) * c5).sin()
                        + 1.0
                }
            }

            fn back_in(self) -> Self {
                let p = $T::clamp(self);
                let s: $T = 1.70158;
                p * p * ((s + 1.0) * p - s)
            }

            fn back_out(self) -> Self {
                let p = $T::clamp(self);
                let s: $T = 1.70158;
                let f = p - 1.0;
                f * f * ((s + 1.0) * f + s) + 1.0
            }

            fn back_in_out(self) -> Self {
                let p = $T::clamp(self);
                let s: $T = 1.70158 * 1.525; // Overshoot adjustment for in-out
                if p < 0.5 {
                    let t = 2.0 * p;
                    0.5 * (t * t * ((s + 1.0) * t - s))
                } else {
                    let t = 2.0 * p - 2.0;
                    0.5 * (t * t * ((s + 1.0) * t + s) + 2.0)
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

#[cfg(test)]
mod tests {
    use super::{Ease, Interpolation};

    // All interpolation variants to verify endpoints.
    const ALL_VARIANTS: [Interpolation; 31] = [
        Interpolation::Linear,
        Interpolation::QuadraticIn,
        Interpolation::QuadraticOut,
        Interpolation::QuadraticInOut,
        Interpolation::CubicIn,
        Interpolation::CubicOut,
        Interpolation::CubicInOut,
        Interpolation::QuarticIn,
        Interpolation::QuarticOut,
        Interpolation::QuarticInOut,
        Interpolation::QuinticIn,
        Interpolation::QuinticOut,
        Interpolation::QuinticInOut,
        Interpolation::SineIn,
        Interpolation::SineOut,
        Interpolation::SineInOut,
        Interpolation::CircularIn,
        Interpolation::CircularOut,
        Interpolation::CircularInOut,
        Interpolation::ExponentialIn,
        Interpolation::ExponentialOut,
        Interpolation::ExponentialInOut,
        Interpolation::ElasticIn,
        Interpolation::ElasticOut,
        Interpolation::ElasticInOut,
        Interpolation::BackIn,
        Interpolation::BackOut,
        Interpolation::BackInOut,
        Interpolation::BounceIn,
        Interpolation::BounceOut,
        Interpolation::BounceInOut,
    ];

    fn assert_approx_eq_f32(a: f32, b: f32, variant: Interpolation, endpoint: &str) {
        let eps = 1e-5_f32;
        assert!(
            (a - b).abs() <= eps,
            "variant={:?} endpoint={} expected {}, got {} (|diff|={})",
            variant,
            endpoint,
            b,
            a,
            (a - b).abs()
        );
    }

    fn assert_approx_eq_f64(a: f64, b: f64, variant: Interpolation, endpoint: &str) {
        let eps = 1e-12_f64;
        assert!(
            (a - b).abs() <= eps,
            "variant={:?} endpoint={} expected {}, got {} (|diff|={})",
            variant,
            endpoint,
            b,
            a,
            (a - b).abs()
        );
    }

    #[test]
    fn endpoints_are_0_and_1_for_f32() {
        for &e in &ALL_VARIANTS {
            let at_0 = 0.0f32.interpolate(e);
            let at_1 = 1.0f32.interpolate(e);
            assert_approx_eq_f32(at_0, 0.0, e, "0");
            assert_approx_eq_f32(at_1, 1.0, e, "1");
        }
    }

    #[test]
    fn endpoints_are_0_and_1_for_f64() {
        for &e in &ALL_VARIANTS {
            let at_0 = 0.0f64.interpolate(e);
            let at_1 = 1.0f64.interpolate(e);
            assert_approx_eq_f64(at_0, 0.0, e, "0");
            assert_approx_eq_f64(at_1, 1.0, e, "1");
        }
    }
}

// The charting code depends on the `plotters` crate which is a dev-dependency.
// It's only compiled during tests.
#[cfg(test)]
mod manual_charts {
    use super::{Ease, Interpolation};
    use plotters::prelude::*;
    use plotters::series::LineSeries;
    use std::fs;

    const OUT_DIR: &str = "target/ease_charts";
    const WIDTH: u32 = 800;
    const HEIGHT: u32 = 600;
    const SAMPLES: usize = 1024;

    fn variant_name(v: Interpolation) -> &'static str {
        match v {
            Interpolation::Linear => "linear",
            Interpolation::QuadraticIn => "quadratic_in",
            Interpolation::QuadraticOut => "quadratic_out",
            Interpolation::QuadraticInOut => "quadratic_in_out",
            Interpolation::CubicIn => "cubic_in",
            Interpolation::CubicOut => "cubic_out",
            Interpolation::CubicInOut => "cubic_in_out",
            Interpolation::QuarticIn => "quartic_in",
            Interpolation::QuarticOut => "quartic_out",
            Interpolation::QuarticInOut => "quartic_in_out",
            Interpolation::QuinticIn => "quintic_in",
            Interpolation::QuinticOut => "quintic_out",
            Interpolation::QuinticInOut => "quintic_in_out",
            Interpolation::SineIn => "sine_in",
            Interpolation::SineOut => "sine_out",
            Interpolation::SineInOut => "sine_in_out",
            Interpolation::CircularIn => "circular_in",
            Interpolation::CircularOut => "circular_out",
            Interpolation::CircularInOut => "circular_in_out",
            Interpolation::ExponentialIn => "exponential_in",
            Interpolation::ExponentialOut => "exponential_out",
            Interpolation::ExponentialInOut => "exponential_in_out",
            Interpolation::ElasticIn => "elastic_in",
            Interpolation::ElasticOut => "elastic_out",
            Interpolation::ElasticInOut => "elastic_in_out",
            Interpolation::BackIn => "back_in",
            Interpolation::BackOut => "back_out",
            Interpolation::BackInOut => "back_in_out",
            Interpolation::BounceIn => "bounce_in",
            Interpolation::BounceOut => "bounce_out",
            Interpolation::BounceInOut => "bounce_in_out",
        }
    }

    fn draw_variant(v: Interpolation) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(OUT_DIR)?;
        let filename = format!("{}/{}.png", OUT_DIR, variant_name(v));

        let root = BitMapBackend::new(&filename, (WIDTH, HEIGHT)).into_drawing_area();
        root.fill(&WHITE)?;

        let mut chart = ChartBuilder::on(&root)
            .caption(format!("Easing: {}", variant_name(v)), ("sans-serif", 28))
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(40)
            // Many easing functions (back/elastic/bounce) overshoot outside [0,1].
            // Use a slightly expanded y-range to visualize the full curve.
            .build_cartesian_2d(0.0f64..1.0f64, -0.3f64..1.3f64)?;

        chart
            .configure_mesh()
            .x_labels(11)
            .y_labels(11)
            .x_desc("t")
            .y_desc("value")
            .draw()?;

        chart.draw_series(LineSeries::new(
            (0..=SAMPLES).map(|i| {
                let t = i as f64 / SAMPLES as f64;
                (t, t.interpolate(v))
            }),
            &BLUE,
        ))?;

        root.present()?;
        Ok(())
    }

    #[test]
    #[ignore]
    fn generate_ease_charts() -> Result<(), Box<dyn std::error::Error>> {
        let variants = [
            Interpolation::Linear,
            Interpolation::QuadraticIn,
            Interpolation::QuadraticOut,
            Interpolation::QuadraticInOut,
            Interpolation::CubicIn,
            Interpolation::CubicOut,
            Interpolation::CubicInOut,
            Interpolation::QuarticIn,
            Interpolation::QuarticOut,
            Interpolation::QuarticInOut,
            Interpolation::QuinticIn,
            Interpolation::QuinticOut,
            Interpolation::QuinticInOut,
            Interpolation::SineIn,
            Interpolation::SineOut,
            Interpolation::SineInOut,
            Interpolation::CircularIn,
            Interpolation::CircularOut,
            Interpolation::CircularInOut,
            Interpolation::ExponentialIn,
            Interpolation::ExponentialOut,
            Interpolation::ExponentialInOut,
            Interpolation::ElasticIn,
            Interpolation::ElasticOut,
            Interpolation::ElasticInOut,
            Interpolation::BackIn,
            Interpolation::BackOut,
            Interpolation::BackInOut,
            Interpolation::BounceIn,
            Interpolation::BounceOut,
            Interpolation::BounceInOut,
        ];

        for &v in &variants {
            draw_variant(v)?;
        }
        Ok(())
    }
}
