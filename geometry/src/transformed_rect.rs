use crate::{Rect, Size, Transform};

/// A rectangle of `size` at the local origin and the transform that places it. The transform is
/// pivoted at the rectangle's center.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TransformedRect {
    pub transform: Transform,
    pub size: Size,
}

impl TransformedRect {
    pub fn new(transform: Transform, size: impl Into<Size>) -> Self {
        Self {
            transform,
            size: size.into(),
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
