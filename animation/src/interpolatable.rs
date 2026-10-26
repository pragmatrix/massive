use std::time::Instant;

use massive_geometry::{Color, PixelCamera, Point, Rect, Size, SizedTransform, Transform, Vector3};

/// For now we have to support `Clone`.
///
/// Other options: We pass 1.0 here and expect `Self` to return a clone for `to`, but can then never
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

// 3D Geometry

impl Interpolatable for Vector3 {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let x = interpolate(&from.x, &to.x, t);
        let y = interpolate(&from.y, &to.y, t);
        let z = interpolate(&from.z, &to.z, t);
        (x, y, z).into()
    }
}

impl Interpolatable for Transform {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        Transform {
            translate: interpolate(&from.translate, &to.translate, t),
            rotate: from.rotate.slerp(to.rotate, t),
            scale: interpolate(&from.scale, &to.scale, t),
        }
    }
}

impl Interpolatable for SizedTransform {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        SizedTransform::new(
            interpolate(&from.size, &to.size, t),
            interpolate(&from.transform, &to.transform, t),
        )
    }
}

impl Interpolatable for PixelCamera {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        PixelCamera {
            look_at: interpolate(&from.look_at, &to.look_at, t),
            distance: interpolate(&from.distance, &to.distance, t),
            fovy: interpolate(&from.fovy, &to.fovy, t),
        }
    }
}

impl Interpolatable for Size {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let width = interpolate(&from.width, &to.width, t);
        let height = interpolate(&from.height, &to.height, t);
        Size::new(width, height)
    }
}

/// Interpolates each component in sRGB. A short fade between two colors does not need a perceptual
/// color space.
impl Interpolatable for Color {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        Color {
            red: interpolate(&from.red, &to.red, t),
            green: interpolate(&from.green, &to.green, t),
            blue: interpolate(&from.blue, &to.blue, t),
            alpha: interpolate(&from.alpha, &to.alpha, t),
        }
    }
}

// 2D Geometry

impl Interpolatable for Point {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let x = f64::interpolate(&from.x, &to.x, t);
        let y = f64::interpolate(&from.y, &to.y, t);
        (x, y).into()
    }
}

impl Interpolatable for Rect {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let (f_min, f_max) = (*from).into();
        let (t_min, t_max) = (*to).into();
        let min = Point::interpolate(&f_min, &t_min, t);
        let max = Point::interpolate(&f_max, &t_max, t);
        (min, max).into()
    }
}

pub fn interpolate<T>(from: &T, to: &T, t: f64) -> T
where
    T: Interpolatable,
{
    T::interpolate(from, to, t)
}
