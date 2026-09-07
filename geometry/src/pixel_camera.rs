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
    /// The distance from the camera's look-at point back along the view axis. `camera_distance()`
    /// is the pixel-perfect distance (where model pixels map 1:1 onto the surface); larger values
    /// dolly the camera back and shrink content. Zoom is a dolly in depth, not a model scale.
    pub distance: f64,
    pub fovy: f64,
}

impl Default for PixelCamera {
    fn default() -> Self {
        Self::look_at(
            Transform::IDENTITY,
            Self::camera_distance(Self::DEFAULT_FOVY),
            Self::DEFAULT_FOVY,
        )
    }
}

impl PixelCamera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    /// The pixel-perfect distance for this camera's field of view.
    fn pixel_perfect_distance(&self) -> f64 {
        Self::camera_distance(self.fovy)
    }

    /// The pixel-perfect camera distance for a field of view: the distance at which model pixels
    /// map 1:1 onto the surface.
    pub fn camera_distance(fovy: f64) -> f64 {
        1.0 / (fovy / 2.0).to_radians().tan()
    }

    /// Create a new camera from a transform, a resolved distance, and field of view.
    ///
    /// `distance == camera_distance(fovy)` is pixel-perfect; larger values dolly back and zoom out.
    pub fn look_at(look_at: Transform, distance: f64, fovy: f64) -> Self {
        Self {
            look_at,
            distance,
            fovy,
        }
    }

    pub fn with_distance(mut self, distance: f64) -> Self {
        self.distance = distance;
        self
    }

    /// The matrix that moves the model so that the camera target is at 0,0. World-only: the
    /// pixel-perfect distance and any dolly live in [`ndc_camera_move`], kept separate from
    /// camera-space content (which is not dollied).
    pub fn model_camera_matrix(&self) -> Matrix4 {
        self.look_at.inverse().to_matrix4()
    }

    /// Move the model back along the camera axis so that its pointed-to position is visible at the
    /// camera's distance. World projection dollies by `distance` (which is `camera_distance` when
    /// pixel-perfect); camera-space content uses [`pixel_perfect_ndc_camera_move`] to stay fixed.
    pub fn ndc_camera_move(&self) -> Matrix4 {
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, self.distance))
    }

    /// The [`ndc_camera_move`] at the fixed pixel-perfect distance, used for camera-space content
    /// that must remain on-screen regardless of world zoom.
    pub fn pixel_perfect_ndc_camera_move(&self) -> Matrix4 {
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, self.pixel_perfect_distance()))
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

    /// The camera distance that fits `points` within `surface_size`, computed directly.
    ///
    /// Projects each point through the perspective divide at the pixel-perfect distance to find the
    /// true NDC footprint (accounting for foreshortening of yawed content), then solves the closed
    /// -form dolly distance that scales that footprint into the surface. No bisection solver.
    pub fn fit_distance_for_points(&self, points: &[Vector3], surface_size: SizePx) -> f64 {
        if points.is_empty() {
            return self.pixel_perfect_distance();
        }

        let (surface_width, surface_height) = surface_size.into();
        let half_width = surface_width as f64 * 0.5;
        let half_height = surface_height as f64 * 0.5;
        let to_camera = self.look_at.inverse();
        let camera_distance = self.pixel_perfect_distance();

        // Max NDC x/y footprint (|x|<=1, |y|<=1 is on-screen) after the perspective divide.
        let mut max_ndc_x: f64 = 0.0;
        let mut max_ndc_y: f64 = 0.0;
        for point in points {
            let camera_point = to_camera.transform_point(*point);
            let denominator = camera_distance - camera_point.z;
            if denominator <= 0.0 {
                continue;
            }
            let x = camera_distance * camera_point.x / denominator;
            let y = camera_distance * camera_point.y / denominator;
            max_ndc_x = max_ndc_x.max(x.abs());
            max_ndc_y = max_ndc_y.max(y.abs());
        }

        // NDC half-extent is in units where the surface half-width/half-height are 1.0. The required
        // on-screen fit scale is the reciprocal of the relative footprint; dolly is inversely
        // proportional to that scale (`screen_scale = camera_distance / distance`).
        let fit_scale = (half_width / max_ndc_x).min(half_height / max_ndc_y);
        camera_distance / fit_scale
    }
}
