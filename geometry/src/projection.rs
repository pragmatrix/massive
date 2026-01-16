use crate::Matrix4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    pub aspect: f64,
    pub depth_range: (f64, f64),
}

impl Projection {
    pub fn new(aspect: f64, depth_range: (f64, f64)) -> Self {
        Self {
            aspect,
            depth_range,
        }
    }

    /// Create a perspective projection matrix.
    pub fn perspective_matrix(&self, fovy: f64) -> Matrix4 {
        let (near, far) = self.depth_range;
        Matrix4::perspective_rh(fovy.to_radians(), self.aspect, near, far)
    }
}
