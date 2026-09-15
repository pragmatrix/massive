use serde::{Deserialize, Serialize};
use swash::zeno::Placement;

use massive_geometry::{BoxPx, Color, PointPx, SizePx, Vector3};

use crate::ClipBoxPx;
use crate::engine::ShapingEngineKind;

/// Opaque identifier for a font face within one engine instance.
///
/// A `FaceId` is produced by a shaping engine (see [`crate::ShapingEngine`]) and is only
/// meaningful within the [`crate::FontManager`] (engine instance) that created it; it is not
/// interpreted by anything else. Each engine defines its own id space: the Parley engine uses
/// the fontique `Blob` unique id packed with the face index within that file, the cosmic-text
/// engine uses a sequential face number over its own font database. One engine is active per
/// process, so the spaces never meet; a foreign id degrades to a graceful `font_data` miss
/// (the renderer logs and skips the glyph).
///
/// Rasterization resolves a `FaceId` through the engine/manager that produced it back to
/// concrete font data ([`crate::engine::ShapingEngine::font_data`]).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FaceId(u64);

impl FaceId {
    /// Build a `FaceId` from an engine-specific payload value.
    pub const fn new(payload: u64) -> Self {
        Self(payload)
    }

    /// The engine-local payload value (e.g. the cosmic-text registry index).
    pub const fn payload(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    // Local translation.
    //
    // This is separated from the view transformation because matrix changes are expensive.
    //
    // Architecture: This is probably not anymore true since we use Transforms.
    //
    // Keep z zero and x / y integer for keeping a pixel perfect rendering at the origin
    // position.
    pub translation: Vector3,
    pub metrics: GlyphRunMetrics,
    pub text_color: Color,
    // Robustness: As of cosmic-text version 0.15, this is now included in cache-key of every glyph.
    // we may need to remove it from there and use our own "CacheKey" like struct.
    pub text_weight: TextWeight,
    /// The engine that shaped this run's glyphs. Stamped by the engine itself on its
    /// [`ShapedRun`] output and propagated here; debug-checked at the render boundary.
    pub shaping_engine: ShapingEngineKind,
    pub glyphs: Vec<RunGlyph>,
}

// TODO(parley): The `RunGlyph::pos` y-coordinates are normalized to a Y-Up convention at the
// adapter boundary, because Parley lays out in Y-down. Everything downstream (renderer,
// `GlyphRun::place_glyph`, scene hit-testing) assumes Y-up, so the flip happens here. Consider
// modernizing the whole project to Y-down as a separate cleanup.
//

impl GlyphRun {
    pub fn new(
        translation: impl Into<Vector3>,
        metrics: GlyphRunMetrics,
        text_color: Color,
        text_weight: TextWeight,
        shaping_engine: ShapingEngineKind,
        glyphs: Vec<RunGlyph>,
    ) -> Self {
        Self {
            translation: translation.into(),
            metrics,
            text_color,
            text_weight,
            shaping_engine,
            glyphs,
        }
    }

    pub fn with_color(mut self, text_color: Color) -> Self {
        self.text_color = text_color;
        self
    }

    /// Translate a rasterized glyph's position to the coordinate system of the run.
    ///
    /// The swash `Placement` is in Y-down glyph space (origin = advance origin, `top` = ink
    /// top above the baseline); the returned box is in run space, Y-down screen convention:
    /// `min` = (left, top), `max` = (right, bottom), numerically ordered.
    pub fn place_glyph(&self, glyph: &RunGlyph, placement: &Placement) -> BoxPx {
        let max_ascent = self.metrics.max_ascent as i32;
        let pos = glyph.pos;

        let left = pos.x + placement.left;
        let top = pos.y + max_ascent - placement.top;
        let size = SizePx::new(placement.width, placement.height).cast::<i32>();

        BoxPx::from_origin_and_size(PointPx::new(left, top), size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphRunMetrics {
    /// The maximum ascent in pixels (use ceil()).
    ///
    /// Used for baseline positioning of the rasterized glyphs.
    pub max_ascent: u32,
    /// The maximum descent in pixels.
    ///
    /// Used for height computation.
    pub max_descent: u32,
    /// The pixel width of all the glyphs in the run.
    pub width: u32,
}

impl GlyphRunMetrics {
    pub fn from_float(max_ascent: f32, max_descent: f32, width: f32) -> Self {
        // This should cover all pixels to enable culling (later), use ceil().
        Self {
            max_ascent: max_ascent.ceil() as u32,
            max_descent: max_descent.ceil() as u32,
            width: width.ceil() as u32,
        }
    }

    /// Size of the glyph run in font-size pixels.
    ///
    /// Robustness: A run might start start at a negative pixel position, so size is probably not
    ///   enough. Perhaps a rectangle is needed here.
    pub fn size(&self) -> SizePx {
        (self.width, self.max_ascent + self.max_descent).into()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TextWeight(pub u16);

impl Default for TextWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl TextWeight {
    pub const THIN: Self = Self(100);
    pub const EXTRA_LIGHT: Self = Self(200);
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const MEDIUM: Self = Self(500);
    pub const SEMI_BOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
    pub const EXTRA_BOLD: Self = Self(800);
    pub const BLACK: Self = Self(900);
}

/// A glyph inside a [`GlyphRun`].
#[derive(Debug, Clone, PartialEq)]
pub struct RunGlyph {
    /// The position (left / top) relative to the start of the line in pixel.
    ///
    /// x usually starts with zero (may be negative with negative left side bearings). y is
    /// usually 0 meaning that the glyph "boxes" usually are having the same height.
    ///
    /// This is the left top position of the "advance box" (in typography terms). Cosmic text
    /// uses the term "hit box".
    pub pos: PointPx,
    pub key: GlyphKey,
}

impl RunGlyph {
    pub fn new(pos: impl Into<PointPx>, key: GlyphKey) -> Self {
        Self {
            pos: pos.into(),
            key,
        }
    }
}

/// A glyph key identifying a rasterized glyph.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face_id: FaceId,
    pub glyph_id: u16,
    pub font_size_bits: u32,
    pub weight: TextWeight,
    /// The positioned crop window (sentinel-bounded box). Part of the rasterization identity:
    /// a glyph cropped differently is a different bitmap.
    pub clip_box: ClipBoxPx,
}

impl GlyphKey {
    pub fn new(
        face_id: FaceId,
        glyph_id: u16,
        font_size: f32,
        weight: TextWeight,
        clip_box: ClipBoxPx,
    ) -> Self {
        Self {
            face_id,
            glyph_id,
            font_size_bits: font_size.to_bits(),
            weight,
            clip_box,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ClipBoxPx;

    /// Placement must land at `pos + (placement.left, max_ascent - placement.top)` with the
    /// bitmap's size extending right/down. Guards the corner-vs-size distinction of
    /// `Box2D::new` — passing a size as the max corner inverts every glyph after the first.
    #[test]
    fn place_glyph_maps_placement_to_run_space_box() {
        let glyph = RunGlyph::new(
            PointPx::new(35, 3),
            GlyphKey::new(
                FaceId::new(0),
                42,
                13.0,
                TextWeight::NORMAL,
                ClipBoxPx::UNCLIPPED,
            ),
        );
        let run = GlyphRun::new(
            (0., 0., 0.),
            GlyphRunMetrics {
                max_ascent: 10,
                max_descent: 4,
                width: 100,
            },
            Color::BLACK,
            TextWeight::NORMAL,
            ShapingEngineKind::Parley,
            vec![glyph],
        );

        let placement = Placement {
            left: -2,
            top: 8,
            width: 7,
            height: 12,
        };

        let b = run.place_glyph(&run.glyphs[0], &placement);
        assert_eq!(b.min, PointPx::new(33, 5));
        assert_eq!(b.max, PointPx::new(40, 17));
    }
}
