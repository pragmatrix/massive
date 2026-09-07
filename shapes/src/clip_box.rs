use derive_more::{Deref, From};
use massive_geometry::{BoxPx, PointPx, VectorPx};

/// The crop window in a glyph's local pixel space (origin = advance origin, Y-up).
///
/// A newtype over [`BoxPx`] in euclid convention: `min` is the numerically smaller corner
/// (left, bottom), `max` the larger (right, top). A finite edge clips there; a sentinel edge
/// (`i32::MIN`/`i32::MAX`) allows overflow on that side, uniformly for both axes.
///
/// The origin is the advance origin — the top-left of the cell box, on the baseline. X
/// increases right, Y increases upward, matching the glyph ink-box frame.
///
/// The wrapped box is private: construct via `From<BoxPx>` (or [`Self::UNCLIPPED`]), read
/// through `Deref`. Mutation goes through [`Self::widen_left_to`], the only legal change.
#[derive(Debug, Deref, From, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ClipBoxPx(BoxPx);

impl ClipBoxPx {
    /// A crop window with no clipping on any edge (full overflow).
    pub const UNCLIPPED: Self = Self(BoxPx::new(
        PointPx::new(i32::MIN, i32::MIN),
        PointPx::new(i32::MAX, i32::MAX),
    ));

    /// Move both edges by `offset`. Sentinels stay sentinel: a shifted sentinel edge means
    /// overflow is still allowed on that side, so translation never manufactures a clip edge.
    #[must_use]
    pub fn translate(&self, offset: VectorPx) -> Self {
        let shift = |v: i32, d: i32| {
            if v == i32::MIN || v == i32::MAX {
                v
            } else {
                v + d
            }
        };
        Self(BoxPx::new(
            PointPx::new(shift(self.min.x, offset.x), shift(self.min.y, offset.y)),
            PointPx::new(shift(self.max.x, offset.x), shift(self.max.y, offset.y)),
        ))
    }

    /// Extend (only) the left edge to `x` when that is further left. Used for ink that draws
    /// backward over a vacated cell; never narrows an existing window.
    pub fn widen_left_to(&mut self, x: i32) {
        if self.0.min.x != i32::MIN {
            self.0.min.x = self.0.min.x.min(x);
        }
    }
}
