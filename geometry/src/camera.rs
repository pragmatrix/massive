use crate::{scalar, Matrix4, Point3, Vector3};

// TODO: May use yaw / pitch based camera?
// <https://sotrh.github.io/learn-wgpu/intermediate/tutorial12-camera/#the-camera>

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Vector3,
    pub fovy: scalar,
}

impl Camera {
    pub const DEFAULT_FOVY: scalar = 45.0;

    pub fn new(eye: Point3, target: Point3) -> Self {
        Self {
            eye,
            target,
            up: Vector3::unit_y(),
            fovy: Self::DEFAULT_FOVY,
        }
    }

    pub fn view_matrix(&self) -> Matrix4 {
        Matrix4::look_at_rh(self.eye, self.target, self.up)
    }
}
