//! Engine-neutral shaped-glyph data model and the shaping engine contract.
//!
//! This is the seam between [`crate::FontManager`] and the concrete shaping engines (Parley,
//! cosmic-text): engines translate their own layout output into [`ShapedRun`]s of
//! engine-neutral [`ShapedGlyph`]s, and the shared data model (`GlyphRun`, `GlyphKey`) stays
//! engine-agnostic. Rasterization is engine-independent (swash) and resolves glyphs through
//! [`ShapingEngine::font_data`].

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use massive_geometry::Color;

use crate::face_metrics::FaceMetrics;
use crate::font_manager::Shaper;
use crate::{ClipBoxPx, FaceId, GlyphKey, GlyphRun, GlyphRunMetrics, RunGlyph, TextWeight};

/// Shared, reference-counted font file bytes.
pub type FontBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// A per-frame shaping context: one [`Shaper`] plus the frame's published registry
/// snapshot.
///
/// Bundled because the two travel together and their borrows must agree: the session takes
/// `&mut` of the manager handle (the `&mut` gate), so the snapshot has to be *cloned out*
/// before the session opens — holding them as separate parameters forced every call site
/// into that ordering manually (and mt's `cluster_to_run` over the clippy argument limit).
pub struct FrameShaper<'a> {
    /// The session; borrows the manager clone exclusively for the bundle's lifetime.
    pub session: Shaper<'a>,
    /// The manager's published registry snapshot, read before the session opened. Faces
    /// minted by *this* frame's fallbacks may be absent — mint-time publication covers the
    /// concurrent-submission contract, but the snapshot is only guaranteed to carry faces
    /// minted up to session open.
    pub registry: Arc<FontRegistry>,
}

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

/// An immutable snapshot of an engine's [`FaceId`] → font-data registry, published as a
/// shared `Arc` so lock-free readers (the renderer's rasterization path) resolve faces
/// without touching the manager's mutex. Engines rebuild the snapshot after every registry
/// mutation; see [`ShapingEngine::font_registry`].
///
/// The snapshot also carries swash [`FaceMetrics`] per face, extracted eagerly at
/// face-mint time (ADR 0006): glyph-placement consumers read metrics through the same
/// lock-free snapshot instead of parsing swash tables per cluster per frame.
///
/// Font data and metrics live behind one shared inner `Arc`: they are always built from
/// the same entries in one mint, updated together, and read together — so the face sets
/// cannot drift apart, and a reader holding one map implicitly pins the other. (The
/// alternative — two sibling `Arc` maps — made a cheaper `metrics_arc()`-style read
/// possible, but that read runs once per frame, not per cluster, and the split kept the
/// "same face set" invariant as an unchecked manual obligation.)
#[derive(Clone, Default)]
pub struct FontRegistry {
    inner: Arc<RegistryMap>,
}

/// The registry's owned maps — held behind [`FontRegistry`]'s one shared `Arc`.
#[derive(Clone, Default)]
struct RegistryMap {
    fonts: HashMap<FaceId, FontData>,
    metrics: HashMap<FaceId, FaceMetrics>,
}

impl FontRegistry {
    /// A snapshot from minted registry entries (font data + extracted metrics).
    /// Engines call this on publish; readers resolve through [`Self::font_data`] /
    /// [`Self::metrics`].
    pub fn new(
        fonts: Arc<HashMap<FaceId, FontData>>,
        metrics: Arc<HashMap<FaceId, FaceMetrics>>,
    ) -> Self {
        // Extract from the shared Arcs into the owned combined map: the inner `Arc` is
        // born here once, and later snapshots replace it whole.
        Self::from_owned((*fonts).clone(), (*metrics).clone())
    }

    /// Assemble a snapshot from owned maps (the engines' publish path).
    pub(crate) fn from_owned(
        fonts: HashMap<FaceId, FontData>,
        metrics: HashMap<FaceId, FaceMetrics>,
    ) -> Self {
        Self {
            inner: Arc::new(RegistryMap { fonts, metrics }),
        }
    }

    /// Resolve [`FaceId`] to concrete font data for rasterization.
    pub fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.inner.fonts.get(&id).cloned()
    }

    /// The face's published swash metrics, extracted at mint time.
    pub fn metrics(&self, id: FaceId) -> Option<&FaceMetrics> {
        self.inner.metrics.get(&id)
    }

    /// The number of faces in this snapshot — the cosmic epoch-pull token (ADR 0006).
    pub fn face_count(&self) -> usize {
        self.inner.fonts.len()
    }

    /// Every `(FaceId, FontData)` of the snapshot, ordered by face payload.
    ///
    /// Ordering matters only where payload adjacency is meaningful (the cosmic registry's
    /// indexes are sequential); sorting keeps per-clone seed replay deterministic across
    /// `HashMap` iteration orders.
    pub fn entries(&self) -> Vec<(FaceId, FontData)> {
        let mut entries: Vec<_> = self
            .inner
            .fonts
            .iter()
            .map(|(id, data)| (*id, data.clone()))
            .collect();
        entries.sort_unstable_by_key(|(id, _)| std::cmp::Reverse(id.payload()));
        entries.reverse();
        entries
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
            #[cfg(feature = "cosmic-text")]
            "cosmic-text" => Some(Self::CosmicText),
            #[cfg(feature = "parley")]
            "parley" => Some(Self::Parley),
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
    /// The cluster's typographic advance in pixels — the distance its text moves the line
    /// origin, independent of its visual placement. Engines position RTL clusters
    /// *visually*, so `x` is not monotonic in cluster order; anything that needs a
    /// slice's width (attribute segmentation, culling) must sum [`ShapedCluster::advance`]
    /// instead of differencing origins.
    pub advance: f32,
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

    /// Publish the current [`FaceId`] → font-data registry as a shared snapshot.
    ///
    /// Called by the manager after every registry mutation (`load_font`, lazy fallback
    /// interning) so readers — the renderer's rasterization path — resolve faces through
    /// the returned `Arc` without ever taking the manager's mutex. Engines keep registry
    /// ownership; the manager is only a publisher.
    fn font_registry(&self) -> Arc<FontRegistry>;

    /// Shape a single line (the first line of `request.text`) at `font_size` pixels.
    ///
    /// Mint-time method only — the canonical engine inside [`crate::FontManager`] never
    /// shapes per frame (ADR 0006: per-clone scratch contexts shape; the canonical engine
    /// loads fonts and, for cosmic, seeds them). Kept on the trait because the manager's
    /// tests shape through it.
    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun>;

    /// Create this engine's per-handle shape-ready scratch (ADR 0006), seeded from the
    /// `published` snapshot's world. Called by the manager on a handle's first session
    /// open; the scratch then epoch-syncs itself on every open
    /// ([`EngineScratch::sync`]) without touching the manager mutex.
    fn new_scratch(&self, published: &FontRegistry) -> Box<dyn EngineScratch>;

    /// Mint a face from font data into the canonical registry, returning its [`FaceId`]
    /// (ADR 0006: the manager is the only issuer). Used by cosmic fallback interning;
    /// parley never needs it (its `FaceId`s derive from blob ids over the shared
    /// collection, static after every registration).
    fn mint_face(&mut self, data: FontData) -> Option<FaceId> {
        let _ = data;
        None
    }
}

/// Per-handle, engine-specific shaping state behind an engine-neutral contract (ADR 0006).
///
/// [`crate::FontManager`] holds `Box<dyn EngineScratch>` per handle and names no engine
/// type after construction: each engine creates its own scratch via
/// [`ShapingEngine::new_scratch`] (seeded from the published snapshot), keeps it epoch-synced
/// in [`EngineScratch::sync`], and shapes through [`EngineScratch::shape`]. Fallback faces
/// a scratch mints route through the `mint_face` callback — the manager stays the single
/// [`FaceId`] issuer — and engines never see the manager type itself.
pub trait EngineScratch: Send {
    /// Bring the scratch in line with the `published` world (ADR 0006). A no-op for engines
    /// whose state self-syncs (parley, via fontique's shared collection); an epoch-pull for
    /// cosmic (a face-count mismatch re-syncs from the snapshot). Called at every session
    /// open; the manager mutex is untouched.
    fn sync(&mut self, published: &FontRegistry);

    /// Shape one attributed line (the first line of `request.text`) at `font_size` pixels.
    ///
    /// `mint_face` interns an unregistered fallback face into the manager's canonical
    /// registry (the single [`FaceId`] issuer); engines that never mint ignore it.
    fn shape(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        mint_face: &mut dyn FnMut(FontData) -> Option<FaceId>,
    ) -> Option<ShapedRun>;
}

/// Assemble a [`GlyphRun`] from `clusters` of a shaped line.
///
/// The run carries `text_color` / `default_weight`; positions are baseline-relative with
/// positive y below the baseline, matching the downstream convention (see `GlyphRun`).
///
/// `clusters` is a view into [`ShapedRun::clusters`] — the whole line, or a contiguous slice
/// of it (attribute segmentation) — and `width` is the advance spanned by that slice: the
/// run's `width` for the whole line, or (for a slice) the sum of the slice's
/// [`ShapedCluster::advance`]s. Engine-independent: engines position RTL clusters
/// *visually*, so origin differences (`next.x − first.x`) are not direction-safe.
pub fn shaped_run_to_glyph_run(
    run: &ShapedRun,
    clusters: &[ShapedCluster],
    width: f32,
    text_color: Color,
    default_weight: TextWeight,
    translation: massive_geometry::Vector3,
) -> GlyphRun {
    let mut glyphs = Vec::with_capacity(clusters.iter().map(|c| c.glyph_range.len()).sum());
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
