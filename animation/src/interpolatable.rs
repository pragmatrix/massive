use std::time::Instant;

use massive_geometry::{PixelCamera, Point, Rect, Size, Transform, Vector3};

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

impl Interpolatable for PixelCamera {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let look_at = interpolate(&from.look_at, &to.look_at, t);
        let target_size = interpolate(&from.target_size, &to.target_size, t);
        let fovy = interpolate(&from.fovy, &to.fovy, t);
        PixelCamera::look_at(look_at, target_size, fovy)
    }
}

impl Interpolatable for Size {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let width = interpolate(&from.width, &to.width, t);
        let height = interpolate(&from.height, &to.height, t);
        Size::new(width, height)
    }
}

impl<T: Interpolatable> Interpolatable for Option<T> {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        match (from, to) {
            (Some(from), Some(to)) => Some(T::interpolate(from, to, t)),
            // Snap to the target value when transitioning between Some and None
            (None, None) => None,
            (Some(_), None) | (None, Some(_)) => to.clone(),
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
