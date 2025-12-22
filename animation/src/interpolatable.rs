use std::time::Instant;

use massive_geometry::{PixelCamera, Transform, Vector3};

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

impl Interpolatable for Vector3 {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let x = f64::interpolate(&from.x, &to.x, t);
        let y = f64::interpolate(&from.y, &to.y, t);
        let z = f64::interpolate(&from.z, &to.z, t);
        (x, y, z).into()
    }
}

impl Interpolatable for Transform {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        Transform {
            translate: Vector3::interpolate(&from.translate, &to.translate, t),
            rotate: from.rotate.slerp(to.rotate, t),
            scale: f64::interpolate(&from.scale, &to.scale, t),
        }
    }
}

impl Interpolatable for PixelCamera {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let interpolated_transform = Transform::interpolate(&from.look_at, &to.look_at, t);
        let fovy = f64::interpolate(&from.fovy, &to.fovy, t);
        PixelCamera::look_at(interpolated_transform, fovy)
    }
}
