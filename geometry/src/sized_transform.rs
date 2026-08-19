use crate::{Rect, Size, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizedTransform {
    pub size: Size,
    pub transform: Transform,
}

impl SizedTransform {
    pub fn new(size: impl Into<Size>, transform: Transform) -> Self {
        Self {
            size: size.into(),
            transform,
        }
    }

    pub fn rect(self) -> Rect {
        self.size.to_rect()
    }

    pub fn to_origin_space(self) -> Transform {
        self.transform.to_origin_space(self.rect().center())
    }
}
