use crate::Matrix4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    pub aspect: f64,
    pub near: f64,
    pub far: f64,
}

impl Projection {
    pub fn new(aspect: f64, near: f64, far: f64) -> Self {
        Self { aspect, near, far }
    }

    /// Create a perspective projection matrix.
    pub fn perspective_matrix(&self, fovy: f64) -> Matrix4 {
        cgmath::perspective(cgmath::Deg(fovy), self.aspect, self.near, self.far)
    }
}
