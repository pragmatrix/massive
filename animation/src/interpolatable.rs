use std::time::Instant;

use massive_geometry::{
    Color, PixelCamera, Point, Quaternion, Rect, Size, SizedTransform, Transform, Vector3,
};

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

/// A transform that only scales and moves zooms about the one point that both ends map alike. On
/// screen, transforms nest: a slot's in its matrix's, and a matrix's in its project's. A child that
/// grows while its parent shrinks would otherwise drift sideways and back, even when both ends show
/// it at the same place.
impl Interpolatable for Transform {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        if from.rotate == to.rotate {
            return *from * zoom_toward(from.inverse() * *to, t);
        }
        Transform {
            translate: interpolate(&from.translate, &to.translate, t),
            rotate: from.rotate.slerp(to.rotate, t),
            scale: interpolate_ratio(from.scale, to.scale, t),
        }
    }
}

/// Interpolates the transforms pivoted at their origins, because the pivot at the center moves when
/// the size changes. The zoom about a fixed point needs both ends in the same coordinates.
impl Interpolatable for SizedTransform {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        let size = interpolate(&from.size, &to.size, t);
        let origin = interpolate(&from.pivoted_at_origin(), &to.pivoted_at_origin(), t);
        SizedTransform::new(size, origin.pivoted_at(size.center()))
    }
}

impl Interpolatable for PixelCamera {
    fn interpolate(from: &Self, to: &Self, t: f64) -> Self {
        PixelCamera {
            look_at: interpolate(&from.look_at, &to.look_at, t),
            distance: interpolate_ratio(from.distance, to.distance, t),
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

/// Interpolates a scale factor, such as a transform's scale or a camera's distance, in equal ratios
/// instead of equal differences. On screen, scales multiply: a child's scale with its parent's, and
/// a layout's scale with the camera's zoom. Two linear interpolations that move in opposite
/// directions multiply to a size that swells and shrinks back on the way. Two ratio interpolations
/// with the same progress multiply to another ratio interpolation, so their product never
/// overshoots its ends.
fn interpolate_ratio(from: f64, to: f64, t: f64) -> f64 {
    from.powf(1.0 - t) * to.powf(t)
}

/// The part `t` of the way from the identity to `step`, a transform that scales and moves but does
/// not rotate. It scales by ratio about the point `step` keeps in place. The translation's share
/// is computed with `exp_m1`, so a scale near 1, whose fixed point lies far away, stays precise,
/// and a scale of exactly 1 moves linearly.
fn zoom_toward(step: Transform, t: f64) -> Transform {
    let log_scale = step.scale.ln();
    let share = if log_scale == 0.0 {
        t
    } else {
        (t * log_scale).exp_m1() / log_scale.exp_m1()
    };
    Transform::new(
        step.translate * share,
        Quaternion::IDENTITY,
        (t * log_scale).exp(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When a group is dissolved, its slots grow from half size to full size while their parent
    /// shrinks from full size to half size. Their size on screen, the product of both scales, stays
    /// between its start and its end throughout.
    #[test]
    fn opposite_scales_multiply_without_overshoot() {
        let (child_from, child_to) = (0.5, 1.0);
        let (parent_from, parent_to) = (1.0, 0.5);
        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let product = interpolate_ratio(child_from, child_to, t)
                * interpolate_ratio(parent_from, parent_to, t);
            assert!((product - 0.5).abs() < 1e-12, "{product} at {t}");
        }
    }

    /// A transform that halves its scale and moves zooms about the point it keeps in place, so a
    /// child laid out in it does not drift away from that point on the way.
    #[test]
    fn a_scaling_transform_zooms_about_its_fixed_point() {
        let from = Transform::IDENTITY;
        let to = Transform::new((50.0, 0.0, 0.0), Quaternion::IDENTITY, 0.5);
        let fixed_point = Vector3::new(100.0, 0.0, 0.0);
        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let mapped = interpolate(&from, &to, t).transform_point(fixed_point);
            assert!((mapped - fixed_point).length() < 1e-9, "{mapped:?} at {t}");
        }
    }
}
