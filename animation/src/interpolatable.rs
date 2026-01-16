use std::time::Instant;

use massive_geometry::{CameraMode, PixelCamera, Point, Rect, Size, Transform, Vector3};

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
        PixelCamera {
            look_at: interpolate(&from.look_at, &to.look_at, t),
            mode: interpolate(&from.mode, &to.mode, t),
            fovy: interpolate(&from.fovy, &to.fovy, t),
        }
    }
}

impl Interpolatable for CameraMode {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        use CameraMode::*;

        match (from, to) {
            (PixelPerfect, PixelPerfect) => PixelPerfect,
            (PixelPerfect, Sized { target_size, .. }) => {
                let blend = interpolate(&0.0, &1.0, t);
                Sized {
                    target_size: *target_size,
                    blend,
                }
            }
            (Sized { target_size, .. }, PixelPerfect) => {
                let blend = interpolate(&1.0, &0.0, t);
                if blend == 0.0 {
                    PixelPerfect
                } else {
                    Sized {
                        target_size: *target_size,
                        blend,
                    }
                }
            }
            (
                Sized {
                    target_size: from_size,
                    blend: from_blend,
                },
                Sized {
                    target_size: to_size,
                    blend: to_blend,
                },
            ) => Sized {
                target_size: interpolate(from_size, to_size, t),
                blend: interpolate(from_blend, to_blend, t),
            },
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
