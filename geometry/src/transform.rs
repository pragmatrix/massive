use std::ops::{Mul, MulAssign};

use crate::{Matrix4, Quaternion, ToVector3, Vector3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translate: Vector3,
    pub rotate: Quaternion,
    pub scale: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translate: Vector3::ZERO,
        rotate: Quaternion::IDENTITY,
        scale: 1.0,
    };

    pub fn new(translate: impl Into<Vector3>, rotate: Quaternion, scale: f64) -> Self {
        Self {
            translate: translate.into(),
            rotate,
            scale,
        }
    }

    pub fn from_translation(translation: impl Into<Vector3>) -> Self {
        translation.into().into()
    }

    pub fn from_rotation(rotation: Quaternion) -> Self {
        rotation.into()
    }

    pub fn from_scale(scale: f64) -> Self {
        scale.into()
    }

    pub fn to_matrix4(&self) -> Matrix4 {
        // Fast path for translation-only transforms
        if self.is_translation_only() {
            return Matrix4::from_translation(self.translate);
        }
        Matrix4::from_scale_rotation_translation(
            Vector3::splat(self.scale),
            self.rotate,
            self.translate,
        )
    }

    pub fn from_matrix4(matrix: Matrix4) -> Self {
        let (scale_vec, rotate, translate) = matrix.to_scale_rotation_translation();
        // Use average scale if non-uniform
        let scale = (scale_vec.x + scale_vec.y + scale_vec.z) / 3.0;
        Self {
            translate,
            rotate,
            scale,
        }
    }

    pub fn transform_point(&self, point: Vector3) -> Vector3 {
        // Fast path for translation-only transforms
        if self.is_translation_only() {
            return point + self.translate;
        }
        self.rotate * (point * self.scale) + self.translate
    }

    pub fn transform_vector(&self, vector: Vector3) -> Vector3 {
        // Fast path for translation-only transforms
        if self.is_translation_only() {
            return vector;
        }
        self.rotate * (vector * self.scale)
    }

    pub fn inverse(&self) -> Self {
        let inv_scale = 1.0 / self.scale;
        let inv_rotate = self.rotate.inverse();
        let inv_translate = inv_rotate * (-self.translate * inv_scale);

        Self {
            translate: inv_translate,
            rotate: inv_rotate,
            scale: inv_scale,
        }
    }

    pub fn is_translation_only(&self) -> bool {
        self.rotate == Quaternion::IDENTITY && self.scale == 1.0
    }

    // Commented, because I don't like it: Who knows in which scale we act.
    // pub fn is_near_identity(&self) -> bool {
    //     // Use approximate comparisons similar to glam's approach
    //     const EPSILON: f64 = 1e-6;
    //     self.translate.abs_diff_eq(Vector3::ZERO, EPSILON)
    //         && self.rotate.is_near_identity()
    //         && (self.scale - 1.0).abs() < EPSILON
    // }
}

impl Mul for Transform {
    type Output = Transform;

    fn mul(self, rhs: Transform) -> Self::Output {
        Transform {
            translate: self.translate + self.rotate * (rhs.translate * self.scale),
            rotate: self.rotate * rhs.rotate,
            scale: self.scale * rhs.scale,
        }
    }
}

impl MulAssign for Transform {
    fn mul_assign(&mut self, rhs: Transform) {
        *self = *self * rhs;
    }
}

impl From<(f64, f64, f64)> for Transform {
    fn from(value: (f64, f64, f64)) -> Self {
        Self::from(Vector3::from(value))
    }
}

impl From<Vector3> for Transform {
    fn from(translate: Vector3) -> Self {
        Self {
            translate,
            ..Default::default()
        }
    }
}

impl<U> From<euclid::Vector3D<f64, U>> for Transform {
    fn from(value: euclid::Vector3D<f64, U>) -> Self {
        value.to_vector3().into()
    }
}

impl From<Quaternion> for Transform {
    fn from(rotate: Quaternion) -> Self {
        Self {
            rotate,
            ..Default::default()
        }
    }
}

impl From<f64> for Transform {
    fn from(scale: f64) -> Self {
        Self {
            scale,
            ..Default::default()
        }
    }
}
