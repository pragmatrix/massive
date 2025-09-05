use crate::{Matrix4, Point3, Projection, Vector3};

// TODO: May use yaw / pitch based camera?
// <https://sotrh.github.io/learn-wgpu/intermediate/tutorial12-camera/#the-camera>

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Vector3,
    pub fovy: f64,
}

impl Camera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    pub fn new(eye: impl Into<Point3>, target: impl Into<Point3>) -> Self {
        Self {
            eye: eye.into(),
            target: target.into(),
            up: Vector3::unit_y(),
            fovy: Self::DEFAULT_FOVY,
        }
    }

    pub fn view_matrix(&self) -> Matrix4 {
        Matrix4::look_at_rh(self.eye, self.target, self.up)
    }

    pub fn view_projection_matrix(&self, z_range: (f64, f64), surface_size: (u32, u32)) -> Matrix4 {
        let (width, height) = surface_size;
        let projection = Projection::new(width as f64 / height as f64, z_range.0, z_range.1);
        view_projection_matrix(self, &projection)
    }
}

pub fn view_projection_matrix(camera: &Camera, projection: &Projection) -> Matrix4 {
    let view = camera.view_matrix();
    let proj = projection.perspective_matrix(camera.fovy);
    OPENGL_TO_WGPU_MATRIX * proj * view
}

// Convert from a projection (OpenGL) from a left handed coordinate system to a right handed
// coordinate system (WGPU).
// <https://sotrh.github.io/learn-wgpu/intermediate/tutorial12-camera/#the-camera>
#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4 = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);
