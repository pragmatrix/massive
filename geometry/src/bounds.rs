use crate::{Point, Rect};

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}

impl Bounds {
    pub fn new(min: impl Into<Point>, max: impl Into<Point>) -> Self {
        Self {
            min: min.into(),
            max: max.into(),
        }
    }

    pub fn to_rect(&self) -> Rect {
        Rect::from(*self)
    }
}

impl From<Bounds> for Rect {
    fn from(b: Bounds) -> Self {
        (b.min, b.max).into()
    }
}

pub trait BoundaryRect {
    fn bounds(self) -> Option<Rect>;
}

impl<T> BoundaryRect for T
where
    T: Iterator<Item = Rect>,
{
    fn bounds(self) -> Option<Rect> {
        self.fold(None, |current, r| {
            Some(match current {
                Some(c) => Rect::join(c, r),
                None => r,
            })
        })
    }
}
