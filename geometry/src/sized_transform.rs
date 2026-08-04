use crate::{Rect, Size, SizePx, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizedTransform {
    pub size: Size,
    pub transform: Transform,
}

impl SizedTransform {
    pub const fn new(size: Size, transform: Transform) -> Self {
        Self { size, transform }
    }

    pub fn from_pixels(size: SizePx, transform: Transform) -> Self {
        Self::new(Size::new(size.width as f64, size.height as f64), transform)
    }

    pub fn rect(self) -> Rect {
        self.size.to_rect()
    }

    pub fn to_origin_space(self) -> Transform {
        self.transform.to_origin_space(self.rect().center())
    }
}
