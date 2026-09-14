//! Engine-neutral shaped-glyph data model and the shaping engine contract.
//!
//! This is the seam between [`crate::FontManager`] and the concrete shaping engines (Parley,
//! cosmic-text): engines translate their own layout output into [`ShapedRun`]s of
//! engine-neutral [`ShapedGlyph`]s, and the shared data model (`GlyphRun`, `GlyphKey`) stays
//! engine-agnostic. Rasterization is engine-independent (swash) and resolves glyphs through
//! [`ShapingEngine::font_data`].

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use massive_geometry::Color;

use crate::{ClipBoxPx, FaceId, GlyphKey, GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

/// Shared, reference-counted font file bytes.
pub type FontBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Concrete font data: shared bytes plus the face index within the font file.
///
/// Field-compatible with fontique's `FontData` so swash rasterization can consume it directly.
#[derive(Clone)]
pub struct FontData {
    pub data: FontBytes,
    pub index: u32,
}

impl FontData {
    pub fn new(data: FontBytes, index: u32) -> Self {
        Self { data, index }
    }
}

/// The font family text is shaped with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TextFamily<'a> {
    /// A concrete family name (e.g. `"JetBrains Mono"`).
    Named(Cow<'a, str>),
    #[default]
    /// The generic sans-serif family.
    SansSerif,
    /// The generic serif family.
    Serif,
    /// The generic monospace family.
    Monospace,
}

impl<'a> From<&'a str> for TextFamily<'a> {
    fn from(name: &'a str) -> Self {
        Self::Named(Cow::Borrowed(name))
    }
}

impl From<String> for TextFamily<'_> {
    fn from(name: String) -> Self {
        Self::Named(Cow::Owned(name))
    }
}

/// The text attributes shaping honors.
#[derive(Debug, Clone, PartialEq)]
pub struct TextAttributes<'a> {
    pub family: TextFamily<'a>,
    pub weight: TextWeight,
    pub color: Color,
    /// Caller-defined identity for this attribute set, echoed onto shaped clusters when the
    /// request enables the metadata mechanism (`ShapingRequest::with_metadata`); ignored
    /// otherwise, and clusters then read `0`.
    ///
    /// An echoed value resolves to the request range whose attributes carry it, falling back
    /// to the default attributes when no range matches — which is also what uncovered text
    /// echoes, since it reads the defaults' own metadata (0 by [`Default`]). Identities across
    /// one request's ranges must therefore be unique and distinct from the default attributes'
    /// metadata, so every echo resolves to exactly one attribute set. A cluster composed
    /// across two ranges (e.g. base + combining mark) echoes the range covering its first
    /// byte; see [`ShapedCluster::metadata`].
    pub metadata: usize,
}

impl Default for TextAttributes<'_> {
    fn default() -> Self {
        Self {
            family: TextFamily::SansSerif,
            weight: TextWeight::default(),
            color: Color::BLACK,
            metadata: 0,
        }
    }
}

impl<'a> TextAttributes<'a> {
    /// Attributes for a named family.
    pub fn named_family(family: impl Into<TextFamily<'a>>) -> Self {
        Self::default().with_family(family)
    }

    pub fn with_family(mut self, family: impl Into<TextFamily<'a>>) -> Self {
        self.family = family.into();
        self
    }

    pub fn with_weight(mut self, weight: TextWeight) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_metadata(mut self, metadata: usize) -> Self {
        self.metadata = metadata;
        self
    }
}

/// A shaping request: attributed text plus per-range attribute overrides.
///
/// Ranges are byte ranges into `text`. Attributes not overridden by a range fall back to
/// `default_attributes`.
#[derive(Debug)]
pub struct ShapingRequest<'a> {
    pub text: &'a str,
    pub default_attributes: TextAttributes<'a>,
    pub ranges: Vec<(Range<usize>, TextAttributes<'a>)>,
    /// Echo [`TextAttributes::metadata`] onto each [`ShapedCluster`]. Disabled by default:
    /// engines then ignore metadata entirely and clusters read `0`. The disabled path matters
    /// for Parley, where enabling adds per-cluster lookups and feature pushes.
    pub metadata: bool,
}

impl<'a> ShapingRequest<'a> {
    pub fn new(text: &'a str, default_attributes: TextAttributes<'a>) -> Self {
        Self {
            text,
            default_attributes,
            ranges: Vec::new(),
            metadata: false,
        }
    }

    /// Enable metadata echoing for this request.
    pub fn with_metadata(mut self) -> Self {
        self.metadata = true;
        self
    }
}

/// The metadata of the first range covering `byte`, or `default_metadata`.
///
/// Shaping engine helper: engines must not interpret metadata, only propagate it. Ranges are
/// few (single digits per shaped line), so a linear scan is fine.
pub fn covering_metadata(
    ranges: &[(Range<usize>, TextAttributes<'_>)],
    default_metadata: usize,
    byte: usize,
) -> usize {
    ranges
        .iter()
        .find(|(range, _)| range.contains(&byte))
        .map(|(_, attributes)| attributes.metadata)
        .unwrap_or(default_metadata)
}

/// The compiled-in shaping engines, selectable at `FontManager` construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapingEngineKind {
    #[cfg(feature = "parley")]
    Parley,
    #[cfg(feature = "cosmic-text")]
    CosmicText,
}

impl ShapingEngineKind {
    /// All engines compiled into this build.
    pub const fn available() -> &'static [Self] {
        const ALL: &[ShapingEngineKind] = &[
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText,
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley,
        ];
        ALL
    }

    /// Parse a shaping engine name (`"cosmic-text"`, `"parley"`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cosmic-text" if cfg!(feature = "cosmic-text") => Some(Self::CosmicText),
            "parley" if cfg!(feature = "parley") => Some(Self::Parley),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "cosmic-text")]
            Self::CosmicText => "cosmic-text",
            #[cfg(feature = "parley")]
            Self::Parley => "parley",
        }
    }
}

/// The result of shaping one line: clusters plus line metrics in pixels.
///
/// Glyphs are stored in one flat array ([`ShapedRun::glyphs`]) instead of per-cluster `Vec`s:
/// nearly every cluster shapes to a single glyph, so a per-cluster `Vec` spent one allocation
/// (and its header) per cluster for nothing. Clusters reference their glyphs by index range
/// (`ShapedCluster::glyph_range`); `benches/cluster_allocations.rs` measures the allocation
/// effect.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    pub clusters: Vec<ShapedCluster>,
    /// All glyphs of this line's clusters, laid out flat in cluster order.
    pub glyphs: Vec<ShapedGlyph>,
    pub max_ascent: f32,
    pub max_descent: f32,
    /// The total advance width of the line in pixels.
    pub width: f32,
    /// The engine that produced this run. Written by the engine itself inside its own `shape`,
    /// so callers cannot attribute a run to the wrong engine; debug-checked at the render
    /// boundary against the resolving manager.
    pub engine: ShapingEngineKind,
}

impl ShapedRun {
    /// The glyphs of `cluster`, resolved through this run's flat glyph array.
    ///
    /// Infallible for engine-constructed ranges (contiguous and in-bounds by construction);
    /// a wild range panics on the slice index instead of silently aliasing other clusters'
    /// glyphs.
    pub fn cluster_glyphs(&self, cluster: &ShapedCluster) -> &[ShapedGlyph] {
        &self.glyphs[cluster.glyph_range.start as usize..cluster.glyph_range.end as usize]
    }
}

/// A shaped cluster: glyphs sharing one source byte range.
///
/// `x` is the cluster's origin on the shaped line (pixels); [`ShapedGlyph::x`] is relative to
/// this origin, so a glyph's absolute line position is `cluster.x + glyph.x` — this holds
/// regardless of the storage encoding below.
///
/// The cluster does not own its glyphs: `glyph_range` indexes into [`ShapedRun::glyphs`], the
/// line's single flat glyph array (Parley's layout-run encoding). Resolve them through
/// [`ShapedRun::cluster_glyphs`]; the range is constructed contiguously by the engines, so
/// wild ranges surface as slice-index panics rather than silent aliasing.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedCluster {
    pub byte_range: Range<usize>,
    /// The cluster's origin on the shaped line, in pixels.
    pub x: f32,
    /// Index range into [`ShapedRun::glyphs`] holding this cluster's glyphs, in order.
    pub glyph_range: Range<u32>,
    /// The caller metadata covering this cluster's first byte, echoing
    /// [`TextAttributes::metadata`] (the default attributes' when no range covers it). Only
    /// maintained while the request enables the mechanism; otherwise `0`.
    ///
    /// Shaping is untouched by the mechanism, so a cluster *may* cross attribute boundaries
    /// (shaping compositions like base + combining mark); such a cluster resolves by its
    /// first byte — the typographically correct side for composed glyphs. Engines define the
    /// echo identically (see [`covering_metadata`] and the engine implementations).
    pub metadata: usize,
}

/// One shaped glyph in the engine-neutral data model.
///
/// `x`/`y` are relative to the cluster origin ([`ShapedCluster::x`]) and the baseline
/// respectively (positive y = below the baseline), matching the `GlyphRun` convention.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub face_id: FaceId,
    pub font_size: f32,
    pub weight: TextWeight,
    pub x: f32,
    pub y: f32,
}

/// A capability-focused contract every shaping engine honors.
///
/// Engines own their font database and selection/fallback policy; they only promise to shape
/// attributed text into the engine-neutral [`ShapedRun`] model and to resolve the [`FaceId`]s
/// they produce back to concrete font data for rasterization.
pub trait ShapingEngine: Send {
    /// The engine identity (e.g. `"parley"`, `"cosmic-text"`).
    fn name(&self) -> &'static str;

    /// Register a font file (all faces of it) and return one [`FaceId`] per face.
    fn load_font(&mut self, data: FontBytes) -> Vec<FaceId>;

    /// Resolve a [`FaceId`] produced by this engine back to concrete font data.
    fn font_data(&self, id: FaceId) -> Option<FontData>;

    /// Shape a single line (the first line of `request.text`) at `font_size` pixels.
    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun>;
}

/// Assemble a [`GlyphRun`] from `clusters` of a shaped line.
///
/// The run carries `text_color` / `default_weight`; positions are baseline-relative with
/// positive y below the baseline, matching the downstream convention (see `GlyphRun`).
///
/// `clusters` is a view into [`ShapedRun::clusters`] — the whole line, or a contiguous slice
/// of it (attribute segmentation) — and `width` is the advance spanned by that slice: the
/// run's `width` for the whole line, or (for a slice) the next cluster's origin minus the
/// slice's first (the run's `width` when the slice reaches the line's end).
pub fn shaped_run_to_glyph_run(
    run: &ShapedRun,
    clusters: &[ShapedCluster],
    width: f32,
    text_color: Color,
    default_weight: TextWeight,
    translation: massive_geometry::Vector3,
) -> GlyphRun {
    let mut glyphs = Vec::with_capacity(
        clusters
            .iter()
            .map(|c| c.glyph_range.len() as usize)
            .sum::<usize>(),
    );
    for cluster in clusters {
        for glyph in run.cluster_glyphs(cluster) {
            glyphs.push(RunGlyph::new(
                ((cluster.x + glyph.x).round() as i32, glyph.y.round() as i32),
                GlyphKey::new(
                    glyph.face_id,
                    glyph.glyph_id,
                    glyph.font_size,
                    glyph.weight,
                    ClipBoxPx::UNCLIPPED,
                ),
            ));
        }
    }

    GlyphRun::new(
        translation,
        GlyphRunMetrics::from_float(run.max_ascent, run.max_descent, width),
        text_color,
        default_weight,
        run.engine,
        glyphs,
    )
}
