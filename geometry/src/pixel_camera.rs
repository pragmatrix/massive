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
}
