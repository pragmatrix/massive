#[derive(Debug, Copy, Clone)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn set(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
    }

    pub fn set_length_fast(&mut self, length: f32) -> bool {
        set_point_length(true, self, self.x, self.y, length, None)
    }
}

fn set_point_length(
    use_rsqrt: bool,
    pt: &mut Point,
    x: f32,
    y: f32,
    length: f32,
    orig_length: Option<&mut f32>,
) -> bool {
    assert!(!use_rsqrt || orig_length.is_none());

    let (xx, yy) = (x as f64, y as f64);
    let dmag = (xx * xx + yy * yy).sqrt();
    let dscale = length as f64 / dmag;
    let (x, y) = ((x as f64 * dscale) as f32, (y as f64 * dscale) as f32);

    if !x.is_finite() || !y.is_finite() || (x == 0.0 && y == 0.0) {
        *pt = Point::new(0.0, 0.0);
        return false;
    }

    if let Some(orig_length) = orig_length {
        *orig_length = dmag as f32;
    }

    *pt = Point::new(x, y);
    true
}
