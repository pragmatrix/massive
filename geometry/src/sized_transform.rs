use crate::{Rect, Size, Transform};

/// A rectangle of `size` at the local origin and the transform that places it. The transform is
/// pivoted at the rectangle's center.
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

    /// The transform pivoted at the rectangle's origin instead of its center.
    pub fn pivoted_at_origin(self) -> Transform {
        self.transform.pivoted_at_origin(self.rect().center())
    }
}
