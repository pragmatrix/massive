use crate::{Matrix4, Projection, SizePx, Transform, Vector3};

/// A pixel camera.
///
/// Detail: The camera is not expressed as its position, but at the point it is pointing to in model
/// coordinates.
///
/// Internally the (pixel) model is transformed so that pixel point the camera is looking at is at
/// 0,0, then everything is projected in the NDC (normalized device coordinates), and then the world
/// is moved back so that the surface pixels match the original pixel space.
#[derive(Debug, Clone, PartialEq, Copy)]
pub struct PixelCamera {
    /// The point the camera points at in model / pixel space.
    pub look_at: Transform,
    /// The resolved scale factor: `1.0` is pixel-perfect, other values zoom the model.
    pub scale: f64,
    pub fovy: f64,
}

impl Default for PixelCamera {
    fn default() -> Self {
        Self::look_at(Transform::IDENTITY, 1.0, Self::DEFAULT_FOVY)
    }
}

impl PixelCamera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    /// Create a new camera from a transform, a resolved scale, and field of view.
    ///
    /// `scale == 1.0` is pixel-perfect; other values zoom the model.
    pub fn look_at(look_at: Transform, scale: f64, fovy: f64) -> Self {
        Self {
            look_at,
            scale,
            fovy,
        }
    }

    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// The matrix that moves and scales the model so that the camera target is at 0,0 and
    /// the target size (if set) fits within the surface.
    pub fn model_camera_matrix(&self) -> Matrix4 {
        self.target_scale_matrix() * self.look_at.inverse().to_matrix4()
    }

    /// Move the model further back in NDC coordinate space, so that its pointed-to position is
    /// visible.
    pub fn ndc_camera_move(&self) -> Matrix4 {
        let camera_distance = 1.0 / (self.fovy / 2.0).to_radians().tan();
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, camera_distance))
    }

    /// The matrix that projects NDC 3D coordinates to the final surface coordinates "2D".
    ///
    /// Architecture: If we internally use pixel coordinates, then go through NDC and here back in
    /// "2D". Is there a more direct way?
    pub fn perspective_matrix(
        &self,
        z_range: (f64, f64),
        surface_size: impl Into<SizePx>,
    ) -> Matrix4 {
        let (width, height) = surface_size.into().into();
        Projection::new(width as f64 / height as f64, z_range).perspective_matrix(self.fovy)
    }

    /// The matrix that scales the model to fit the target size within the surface.
    fn target_scale_matrix(&self) -> Matrix4 {
        let scale = self.scale;
        Matrix4::from_scale(Vector3::new(scale, scale, scale))
    }

    /// The largest camera scale whose projected model points fit within `surface_size`.
    ///
    /// Transforms each point into camera space (via `look_at.inverse()`) and solves for the size
    /// scale at which the perspective projection still fits the surface. Unlike fitting an
    /// axis-aligned union rect, this frames the true projected footprint, so yawed/rotated content
    /// doesn't leave empty strips around its silhouette.
    pub fn fit_scale_for_points(&self, points: &[Vector3], surface_size: SizePx) -> f64 {
        if points.is_empty() {
            return self.scale;
        }

        let (surface_width, surface_height) = surface_size.into();
        let surface_width = surface_width as f64;
        let surface_height = surface_height as f64;
        let camera_distance = 1.0 / (self.fovy / 2.0).to_radians().tan();
        let model_to_ndc_scale = 2.0 / surface_height;
        let half_width = surface_width * 0.5;
        let half_height = surface_height * 0.5;
        let to_camera = self.look_at.inverse();

        let fits = |model_scale: f64| {
            let z_scale = model_to_ndc_scale * model_scale;
            for point in points {
                let camera_point = to_camera.transform_point(*point);
                let denominator = camera_distance - z_scale * camera_point.z;
                if denominator <= 0.0 {
                    return false;
                }
                let x = camera_distance * model_scale * camera_point.x / denominator;
                let y = camera_distance * model_scale * camera_point.y / denominator;
                if x.abs() > half_width || y.abs() > half_height {
                    return false;
                }
            }
            true
        };

        // `fits` is monotone-decreasing in scale (bigger scale → bigger content). Find an upper
        // bound that no longer fits, then bisect for the largest fitting scale.
        let mut lo = 0.0;
        let mut hi = self.scale;
        while hi.abs() < 1024.0 && fits(hi) {
            hi *= 2.0;
        }
        for _ in 0..48 {
            let mid = (lo + hi) * 0.5;
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}
