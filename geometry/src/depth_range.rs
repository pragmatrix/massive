#[derive(Debug, Clone, Copy)]
pub struct DepthRange {
    pub near: f64,
    pub far: f64,
}

impl DepthRange {
    pub fn new(near: f64, far: f64) -> Self {
        Self { near, far }
    }
}

impl From<(f64, f64)> for DepthRange {
    fn from((near, far): (f64, f64)) -> Self {
        Self::new(near, far)
    }
}
