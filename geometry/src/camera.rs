use crate::{Matrix4, Projection, Quaternion, SizePx, Transform, Vector3};

/// A camera backed by a Transform (camera-to-world).
/// The camera looks along the negative Z axis of the transform.
#[derive(Debug, Clone, PartialEq, Copy)]
pub struct Camera {
    pub transform: Transform,
    pub fovy: f64,
}

impl Camera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    /// Create a new camera from a transform and field of view.
    pub fn new(transform: Transform, fovy: f64) -> Self {
        Self { transform, fovy }
    }

    /// A pixel aligned camera in which each unit at z=0 maps to a single pixel on the screen.
    pub fn pixel_aligned(fovy: f64) -> Self {
        Self::pixel_aligned_looking_at(Transform::IDENTITY, fovy)
    }

    /// Create a pixel-aligned camera looking at another transform's position.
    /// The camera is positioned so that the target's center is pixel-aligned at the camera's center.
    /// The camera's roll is aligned with the target transform's coordinate system.
    pub fn pixel_aligned_looking_at(target: Transform, fovy: f64) -> Self {
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        let target_position = target.translate;

        // Position camera along the target's Z axis
        let camera_offset = target.rotate * Vector3::new(0.0, 0.0, camera_distance);
        let eye = target_position + camera_offset;

        // Camera looks back at target with same orientation
        let forward = -camera_offset.normalize();
        let rotate = Quaternion::from_rotation_arc(-Vector3::Z, forward) * target.rotate;

        Self::new(Transform::new(eye, rotate, 1.0), fovy)
    }

    pub fn eye(&self) -> Vector3 {
        self.transform.translate
    }

    pub fn target(&self) -> Vector3 {
        let forward = self.transform.rotate * -Vector3::Z;
        self.transform.translate + forward
    }

    pub fn up(&self) -> Vector3 {
        self.transform.rotate * Vector3::Y
    }

    pub fn view_matrix(&self) -> Matrix4 {
        self.transform.inverse().to_matrix4()
    }

    pub fn view_projection_matrix(
        &self,
        z_range: (f64, f64),
        surface_size: impl Into<SizePx>,
    ) -> Matrix4 {
        let (width, height) = surface_size.into().into();
        let projection = Projection::new(width as f64 / height as f64, z_range.0, z_range.1);
        view_projection_matrix(self, &projection)
    }
}

pub fn view_projection_matrix(camera: &Camera, projection: &Projection) -> Matrix4 {
    let view = camera.view_matrix();
    let proj = projection.perspective_matrix(camera.fovy);
    proj * view
}
