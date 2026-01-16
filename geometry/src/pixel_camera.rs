use crate::{Matrix4, Projection, Size, SizePx, Transform, Vector3};

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
    /// The target visible area size in pixels. When `None`, the camera uses 1:1 pixel mapping.
    /// When `Some`, the camera scales to fit this area within the surface, using letterboxing
    /// (fit to constraining dimension), and 1:1 pixel mapping is no longer maintained.
    pub target_size: Option<Size>,
    pub fovy: f64,
}

impl Default for PixelCamera {
    fn default() -> Self {
        Self::look_at(Transform::IDENTITY, None, Self::DEFAULT_FOVY)
    }
}

impl PixelCamera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    /// Create a new camera from a transform, optional target size, and field of view.
    ///
    /// When `target_size` is `None`, the camera uses 1:1 pixel mapping.
    /// When `target_size` is `Some`, the camera scales to fit the target size using letterboxing.
    pub fn look_at(look_at: Transform, target_size: Option<Size>, fovy: f64) -> Self {
        Self {
            look_at,
            target_size,
            fovy,
        }
    }

    /// The matrix that moves and scales the model so that the camera target is at 0,0 and
    /// the target size (if set) fits within the surface.
    pub fn model_camera_matrix(&self, surface_size: SizePx) -> Matrix4 {
        self.target_scale_matrix(surface_size) * self.look_at.inverse().to_matrix4()
    }

    /// Move the model further back in NDC coordinate space, so that its pointed-to position is visible.
    pub fn ndc_camera_move(&self) -> Matrix4 {
        let camera_distance = 1.0 / (self.fovy / 2.0).to_radians().tan();
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, camera_distance))
    }

    /// The matrix that projects NDC 3D coordinates to the final surface coordinates 2D.
    ///
    /// Architecture: If we internally use pixel coordinates, then go through NDC and here back in
    /// 2D. Is there a more direct way?
    pub fn perspective_matrix(
        &self,
        z_range: (f64, f64),
        surface_size: impl Into<SizePx>,
    ) -> Matrix4 {
        let (width, height) = surface_size.into().into();
        Projection::new(width as f64 / height as f64, z_range).perspective_matrix(self.fovy)
    }

    /// The matrix that scales the model to fit the target size within the surface.
    ///
    /// Returns identity if no target size is set.
    fn target_scale_matrix(&self, surface_size: SizePx) -> Matrix4 {
        let scale = self.target_scale(surface_size);
        Matrix4::from_scale(Vector3::new(scale, scale, scale))
    }

    /// Compute the scale factor needed to fit the target size within the surface.
    ///
    /// Returns 1.0 if no target size is set (1:1 pixel mapping).
    /// Uses letterboxing: fits to the constraining dimension so the entire target is visible.
    fn target_scale(&self, surface_size: SizePx) -> f64 {
        let Some(target_size) = self.target_size else {
            return 1.0;
        };

        let (surface_width, surface_height) = surface_size.into();
        let scale_x = surface_width as f64 / target_size.width;
        let scale_y = surface_height as f64 / target_size.height;

        // Use the smaller scale to ensure the entire target fits (letterboxing)
        scale_x.min(scale_y)
    }
}
