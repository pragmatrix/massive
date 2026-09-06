use massive_geometry::{PointPx, SizePx, VectorPx};

/// The crop window in a glyph's local pixel space (origin = advance origin, Y-up).
///
/// A finite edge clips there; a sentinel edge allows overflow on that side. In Y-up space the
/// top edge is `min.y` and the bottom edge is `max.y` (top > bottom), so "no clip on top" is
/// `min.y = i32::MAX` and "no clip on bottom" is `max.y = i32::MIN`. X is not inverted: "no
/// clip on left" is `min.x = i32::MIN`, "no clip on right" is `max.x = i32::MAX`.
///
/// The origin is the advance origin — the top-left of the cell box, on the baseline. X
/// increases right, Y increases upward, matching the glyph ink-box frame.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ClipBoxPx {
    /// Left / top edge. `i32::MIN` = overflow allowed on the left, `i32::MAX` = overflow
    /// allowed on the top.
    pub min: PointPx,
    /// Right / bottom edge. `i32::MAX` = overflow allowed on the right, `i32::MIN` = overflow
    /// allowed on the bottom.
    pub max: PointPx,
}

impl ClipBoxPx {
    /// A crop window with no clipping on any edge (full overflow).
    pub const UNCLIPPED: Self = Self {
        min: PointPx::new(i32::MIN, i32::MAX),
        max: PointPx::new(i32::MAX, i32::MIN),
    };

    /// Move both edges by `offset`. Sentinels stay sentinel: a shifted sentinel edge means
    /// overflow is still allowed on that side, so translation never manufactures a clip edge.
    #[must_use]
    pub fn translate(&self, offset: VectorPx) -> Self {
        let shift = |p: PointPx| {
            if p.x == i32::MIN || p.x == i32::MAX {
                p
            } else {
                PointPx::new(
                    p.x + offset.x,
                    if p.y == i32::MIN || p.y == i32::MAX {
                        p.y
                    } else {
                        p.y + offset.y
                    },
                )
            }
        };
        Self {
            min: shift(self.min),
            max: shift(self.max),
        }
    }

    /// Extend (only) the left edge to `x` when that is further left. Used for ink that draws
    /// backward over a vacated cell; never narrows an existing window.
    pub fn widen_left_to(&mut self, x: i32) {
        if self.min.x != i32::MIN {
            self.min.x = self.min.x.min(x);
        }
    }

    /// Horizontal extent of the window, for finite edges.
    pub fn span_px(&self) -> SizePx {
        SizePx::new((self.max.x - self.min.x) as u32, 0)
    }
}

// `euclid::Point2D` doesn't implement `Ord`, but `GlyphKey` (which embeds a `ClipBoxPx`)
// derives it, so order lexicographically by the two points.
impl PartialOrd for ClipBoxPx {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ClipBoxPx {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.min.x, self.min.y, self.max.x, self.max.y).cmp(&(
            other.min.x,
            other.min.y,
            other.max.x,
            other.max.y,
        ))
    }
}
