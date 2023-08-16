use crate::{scalar, Matrix4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    pub aspect: scalar,
    pub near: scalar,
    pub far: scalar,
}

impl Projection {
    pub fn new(aspect: scalar, near: scalar, far: scalar) -> Self {
        Self { aspect, near, far }
    }

    /// Create a perspective projection matrix.
    pub fn perspective_matrix(&self, fovy: scalar) -> Matrix4 {
        cgmath::perspective(cgmath::Deg(fovy), self.aspect, self.near, self.far)
    }
}
