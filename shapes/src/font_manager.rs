//! The font manager: owns one shaping engine and exposes the stable shaping API.
//!
//! [`FontManager`] owns one shaping engine (selected at construction, ADR 0005: no default
//! engine, a [`FaceId`] is only meaningful for the engine instance that produced it). Its entry
//! surface is deliberately tiny — `load_font` and [`shaper`](FontManager::shaper) — which is
//! what keeps the mutex gate auditable.
//!
//! ## The `&mut` gate
//!
//! Engine state sits behind an `Arc<Mutex<…>>`, but both entry points deliberately take
//! `&mut self`: a [`Shaper`] guard borrows the manager as mutable for its whole session, so the
//! borrow checker guarantees no other engine access — the self-deadlock that used to hang a
//! test — can even be written while a session is open. Font resolution is session-only
//! ([`Shaper::font_data`]); the manager exposes no per-call path that could race or re-lock
//! against a live session.
//!
//! Cross-thread sharing (instance tasks, renderer) is real, so the manager stays `Clone` over
//! the `Arc`; a *deliberate* clone-then-lock is then review-visible code, not a preventable
//! accident.
//!
//! ## The published registry
//!
//! The engine's [`FaceId`] → font-data registry is additionally *published* as an immutable
//! `Arc` snapshot ([`FontManager::published`]), read lock-free on the render path
//! (rasterization). Republishing happens at the mutation points — `load_font` and session
//! end (the cosmic engine lazily interns fallback faces while shaping) — both occur under
//! the manager lock, so a published snapshot never lags a session's mutations. Note: the
//! gate exempts only this read; snapshot readers still never touch the mutex.

use std::fmt;
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, MutexGuard};

use crate::engine::{
    FontData, FontRegistry, ShapedRun, ShapingEngine, ShapingEngineKind, ShapingRequest,
};
#[cfg(feature = "parley")]
use crate::face_metrics::FaceMetricsCache;
#[cfg(feature = "cosmic-text")]
use crate::shaping_engines::CosmicTextEngine;
use crate::shaping_engines::ParleyEngine;
use crate::{FaceId, GlyphRun};

/// A shaping session holding the manager's lock.
///
/// Created by [`FontManager::shaper`]; the guard it carries keeps the selected engine locked for
/// as long as the guard is alive, so multiple shapes can run against the same engine with a
/// single lock acquisition.
pub struct Shaper<'a> {
    /// The manager's engine kind, fixed at construction; read without a lock.
    kind: ShapingEngineKind,
    inner: MutexGuard<'a, FontManagerInner>,
    /// Set when the session may have mutated the engine registry (lazy fallback interning);
    /// dropping the guard republishes the registry snapshot through this back-pointer.
    published: &'a ArcSwap<FontRegistry>,
}

impl Drop for Shaper<'_> {
    fn drop(&mut self) {
        // Republish while the lock is still held: any registry mutation this session made
        // (cosmic fallback interning) becomes visible to lock-free readers only here.
        self.published.store(self.inner.engine.font_registry());
    }
}

impl fmt::Debug for FontManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontManager")
            .field("engine", &self.kind)
            .finish_non_exhaustive()
    }
}

struct FontManagerInner {
    /// Boxed: the engines carry large contexts (~1–2 kB: Parley's `FontContext` +
    /// `LayoutContext`, cosmic-text's `FontSystem`); an extra indirection per call is
    /// negligible next to shaping and keeps the manager handle small.
    engine: Box<dyn ShapingEngine>,
    /// Per-face swash metrics extracted to owned storage, filled on demand via
    /// [`Shaper::face_metrics`] (see `face_metrics`). Lives here so entries share the same
    /// synchronized state the engine's face registry lives under.
    face_metrics: FaceMetricsCache,
}

/// A font manager owning one shaping engine.
///
/// The public API mirrors the pre-engine surface so consumers keep compiling; `shaper()` yields a
/// [`Shaper`] guard through which text is shaped via [`crate::TextShaper`] (the engine-neutral
/// builder).
#[derive(Clone)]
pub struct FontManager {
    /// The engine kind selected at construction. Immutable for the manager's lifetime, so it
    /// reads without the lock; only the engine instance needs synchronized access.
    kind: ShapingEngineKind,
    inner: Arc<Mutex<FontManagerInner>>,
    /// The last-published font registry snapshot (see [`Self::published`]): swapped under the
    /// manager lock at every registry-mutating point, read lock-free on the render path.
    published: Arc<ArcSwap<FontRegistry>>,
}

impl FontManager {
    fn with_engine(kind: ShapingEngineKind, engine: Box<dyn ShapingEngine>) -> Self {
        let published = Arc::new(ArcSwap::from(engine.font_registry()));
        Self {
            kind,
            inner: Arc::new(Mutex::new(FontManagerInner {
                engine,
                face_metrics: FaceMetricsCache::default(),
            })),
            published,
        }
    }

    /// Create a manager over the given engine kind, with system fonts loaded.
    pub fn system(kind: ShapingEngineKind) -> Self {
        let engine: Box<dyn ShapingEngine> = match kind {
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley => Box::new(ParleyEngine::system()),
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText => Box::new(CosmicTextEngine::system()),
        };
        Self::with_engine(kind, engine)
    }

    /// A bare manager over the given engine kind: no fallbacks, no fonts.
    pub fn bare(kind: ShapingEngineKind) -> Self {
        let engine: Box<dyn ShapingEngine> = match kind {
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley => Box::new(ParleyEngine::bare()),
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText => Box::new(CosmicTextEngine::bare()),
        };
        Self::with_engine(kind, engine)
    }

    /// Adds the font and returns Self
    pub fn with_font(mut self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Self {
        self.load_font(font_data);
        self
    }

    /// Adds the font and returns its font ids.
    ///
    /// Together with [`Self::shaper`], this is the *entire* entry surface for engine state,
    /// so the mutex gate is checkable at a glance: both take `&mut self` — deliberately
    /// artificial (the state is behind an `Arc<Mutex<…>>`) — and a [`Shaper`] guard borrows
    /// the manager as mutable, freezing both while the session is open. Wrong interleavings
    /// are compile errors, not hangs.
    pub fn load_font(
        &mut self,
        font_data: impl AsRef<[u8]> + Sync + Send + 'static,
    ) -> Vec<FaceId> {
        let mut inner = self.inner.lock();
        let ids = inner.engine.load_font(Arc::new(font_data));
        // The registry just mutated: republish while the lock is still held (see module doc).
        self.published.store(inner.engine.font_registry());
        ids
    }

    /// The engine this manager shapes with.
    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.kind
    }

    /// The last-published registry snapshot, lock-free.
    ///
    /// The one gate exemption: readers take an `Arc` copy from the [`ArcSwap`] — no manager
    /// mutex is involved, so this cannot deadlock against a live shaper session. Freshness
    /// is guaranteed for everything published before the *current* session ends; mutations
    /// inside a live session become visible when it drops (see [`Shaper`]).
    pub fn published(&self) -> Arc<FontRegistry> {
        self.published.load_full()
    }

    /// Acquire a [`Shaper`], holding the manager's lock for the duration of the shaping session.
    ///
    /// The returned guard borrows the manager as mutable, which is what freezes all other
    /// manager access while the session is open (see the module doc).
    #[must_use]
    pub fn shaper(&mut self) -> Shaper<'_> {
        Shaper {
            kind: self.kind,
            inner: self.inner.lock(),
            published: &self.published,
        }
    }
}

impl Shaper<'_> {
    /// Shape one attributed line at `font_size` through the selected engine.
    pub fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        self.inner.engine.shape(request, font_size)
    }

    /// Resolve concrete font data through the engine of this session.
    ///
    /// The session path is the *only* path: the manager exposes no per-call resolution, so
    /// font identity is always resolved under the same lock acquisition the shapes ran in.
    pub fn font_data(&mut self, id: FaceId) -> Option<FontData> {
        self.inner.engine.font_data(id)
    }

    /// The cached swash metrics of the face `id` was produced by, extracted on first use.
    ///
    /// Glyph placement consumers (cluster grid anchoring, ink checks) read this instead of
    /// constructing a `FontRef` and parsing the metric tables per cluster per frame. Resolution
    /// follows [`Self::font_data`]'s lock discipline: callers holding a shaper guard resolve
    /// through this, never through the manager.
    pub fn face_metrics(&mut self, id: FaceId) -> Option<&crate::FaceMetrics> {
        let font_data = self.inner.engine.font_data(id)?;
        self.inner.face_metrics.entry(id, font_data)
    }

    /// Shape and assemble a [`GlyphRun`] carrying the default attributes' color/weight.
    pub fn glyph_run(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<GlyphRun> {
        let run = self.shape(request, font_size)?;
        let color = request.default_attributes.color;
        let weight = request.default_attributes.weight;
        Some(crate::engine::shaped_run_to_glyph_run(
            &run,
            &run.clusters,
            run.width,
            color,
            weight,
            Default::default(),
        ))
    }

    /// The engine this shaper shapes with.
    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{ShapedCluster, TextAttributes};

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// A bundled font that exercises nonzero vertical glyph offsets, which the other
    /// bundled fixtures (monospace, Montserrat) lack entirely. See the sign test below
    /// for the fixture rationale. OFL 1.1, licensed alongside the font file.
    const TAKRI: &[u8] =
        include_bytes!("../../assets/fonts/NotoSansTakri/NotoSansTakri-Regular.ttf");

    /// A bundled Arabic+Latin font so the bidi/RTL tests don't depend on the system
    /// font database (Arabic coverage is optional on many systems, and system fallback
    /// picks vary per platform). OFL 1.1, licensed alongside the font file.
    const AMIRI: &[u8] = include_bytes!("../../assets/fonts/Amiri/Amiri-Regular.ttf");

    fn all_engines() -> Vec<ShapingEngineKind> {
        ShapingEngineKind::available().to_vec()
    }

    /// Vertical glyph offsets must share one sign convention across engines: positive y =
    /// below the baseline (see `ShapedGlyph::y`). HarfBuzz reports Y-up offsets; Parley
    /// already negates them into Y-down, cosmic-text's `ShapeGlyph` copies them verbatim
    /// (Y-up), so the cosmic engine must negate when translating.
    ///
    /// Fixture: Noto Sans Takri (bundled, OFL 1.1) — the Takri sequence
    /// [U+1168A][U+116B6][U+116A9] shapes through a ccmp chain ligature whose
    /// post-base form receives a nonzero YPlacement through mark attachment, so the
    /// shaped run really exercises a vertical offset (the monospace fixture carries
    /// none at all, which would make the test meaningless).
    #[test]
    fn shaped_y_offsets_share_sign_convention() {
        let text = "\u{1168A}\u{116B6}\u{116A9}";
        let mut by_kind = Vec::new();
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind).with_font(TAKRI);
            let request =
                ShapingRequest::new(text, TextAttributes::named_family("Noto Sans Takri"));
            let run = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&request, 16.0)
                    .expect("shaping must produce a run")
            };
            let ys: Vec<_> = run.glyphs.iter().map(|g| g.y).collect();
            assert!(
                ys.iter().any(|&y| y != 0.0),
                "{kind:?} text {text:?}: the fixture must exercise a nonzero vertical \
                 offset to be a meaningful sign test (glyph ys: {ys:?})"
            );
            by_kind.push((kind, ys));
        }
        // The engine-neutral model must agree, whatever the underlying libraries report.
        // The engine-neutral model must agree on sign AND magnitude, whatever the
        // underlying libraries report. The offset path differs per engine (Parley scales
        // in the font pipeline, cosmic-text scales em-relative afterwards), so allow a
        // small float tolerance here — but nothing near the sign range (±3 px).
        for (kind, ys) in &by_kind[1..] {
            for (i, (a, b)) in by_kind[0].1.iter().zip(ys.iter()).enumerate() {
                assert!(
                    (a - b).abs() < 0.01,
                    "{:?} vs {:?} text {text:?}: ShapedGlyph.y[{i}] diverges ({a} vs {b}); \
                     sign must match across engines",
                    by_kind[0].0,
                    kind
                );
            }
        }
    }

    /// Regression: the cosmic engine must not collapse `max_descent` to 0 — its
    /// accumulator starts at `0.0` and takes `max(font_size * ShapeGlyph.descent)`.
    /// Verified premise: cosmic-text's `ShapeGlyph.descent` is `metrics.descent/upem`
    /// with swash reading `descent = -descender` (positive for typical fonts), so the
    /// max sees a positive value — a lowercase line with real descenders shapes to a
    /// positive run descent. Parity check included: both engines shape the same
    /// fixture from the same font, so engine-divergence (not just collapse) fails.
    #[test]
    fn shaped_run_has_positive_max_descent_on_both_engines() {
        let descenders = "jyg";
        let mut by_kind = Vec::new();
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            let request =
                ShapingRequest::new(descenders, TextAttributes::named_family("JetBrains Mono"));
            let run = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&request, 16.0)
                    .expect("shaping must produce a run")
            };
            assert!(
                run.max_descent > 0.0,
                "{kind:?}: run.max_descent collapsed to {descenders:?}"
            );
            by_kind.push((kind, run.max_descent));
        }
        let (_, first) = by_kind[0];
        for (kind, descent) in &by_kind[1..] {
            assert_eq!(
                (descent * 1000.0).round() as i32,
                (first * 1000.0).round() as i32,
                "{kind:?} diverges from {:?} on max_descent ({descent} vs {first})",
                by_kind[0].0
            );
        }
    }

    /// TEMP diagnostic: dump cluster origins for RTL text to verify the
    /// attribute_runs width bug's premise (non-monotonic ShapedCluster.x).
    #[test]
    #[ignore = "diagnostic probe"]
    fn probe_rtl_dump() {
        for text in ["abسلام", "سلامabسلام", "سلام"] {
            for kind in all_engines() {
                let mut fonts = FontManager::bare(kind).with_font(AMIRI);
                let mut request = ShapingRequest::new(text, TextAttributes::named_family("Amiri"))
                    .with_metadata();
                // Attribute every Arabic segment (defaults cover the LTR islands).
                let salam = "سلام";
                let mut ranges = Vec::new();
                let mut from = 0;
                while let Some(at) = text[from..].find(salam) {
                    let start = from + at;
                    ranges.push((
                        start..start + salam.len(),
                        TextAttributes::default().with_metadata(1),
                    ));
                    from = start + salam.len();
                }
                request.ranges = ranges;
                let run = {
                    let mut shaper = fonts.shaper();
                    shaper
                        .shape(&request, 16.0)
                        .expect("shaping must produce a run")
                };
                println!(
                    "[rtl] {kind:?} {text:?} width={} ascent/descent={}/{}",
                    run.width, run.max_ascent, run.max_descent
                );
                for c in &run.clusters {
                    println!("  bytes={:?} x={} meta={}", c.byte_range, c.x, c.metadata);
                }
            }
        }
    }

    /// Regression: RTL text must be positioned *visually* — the logically-earlier cluster
    /// sits at the larger x (Arabic renders right-to-left).
    ///
    /// Both engines currently violate this (diagnostics: the ignored `probe_rtl_dump`
    /// dumps real origins per engine), so RTL renders mirrored on screen:
    ///
    /// - Parley mirrors RTL *runs* everywhere: `parley_engine::shape` accumulates
    ///   `cluster_origin` in logical order (`cluster_origin += cluster.advance()`,
    ///   direction-blind), so even an RTL run embedded in an LTR paragraph ("abسلام")
    ///   places its logically-first cluster at the run's left edge.
    /// - cosmic-text is correct for an embedded RTL run (its `shape()` emits that word's
    ///   glyphs in visual order), but for an RTL *line* (paragraph direction RTL, e.g.
    ///   pure "سلام") `cosmic-text`'s `shape()` hands glyphs back in *logical* order
    ///   (`shape.rs` reverses harfrust's visual order for `line_rtl`), deferring the
    ///   RTL positioning to cosmic-text's own layout phase — which this engine bypasses.
    ///   The naive left-to-right accumulation then mirrors the whole line.
    ///
    /// Upstream confirmation:
    /// - cosmic-text #113 "BIDI Layout is 'random'"
    ///   (https://github.com/pop-os/cosmic-text/issues/113, open): a consumer with exactly
    ///   this engine's usage (`BufferLine::shape` without the layout phase) reports
    ///   Arabic "either reversed or not" per line direction; CryZe pins it to
    ///   `shape.rs`'s `// Reverse glyphs in RTL lines` (the same `line_rtl:`
    ///   `word.glyphs.reverse()` we probed above) and to the always-positive
    ///   `x_advance`s of logical-order glyphs. Maintainer hojjatabdollahi confirms
    ///   visual positioning lives only in `ShapeLine::layout()`: "The whole point of
    ///   cosmic-text is that it does that for you" — bypassing layout for BiDi is
    ///   unsupported, and moving the reversal into layout is acknowledged as future work.
    /// - cosmic-text #190 "Hebrew words (RTL) are not rendered correctly on main"
    ///   (https://github.com/pop-os/cosmic-text/issues/190, fixed by #191): a regression
    ///   that broke the shape→layout ordering contract and rendered Hebrew mirrored —
    ///   the same failure mode this test pins.
    /// - parley: `Run::clusters()` iterates in logical order while parley renders
    ///   visually (see `Run::visual_clusters()`/`logical_to_visual` in parley's run.rs;
    ///   cf. linebender/parley #298, a cursor-navigation bug over the same ordering
    ///   contract). No open parley issue reports rendered mirroring — parley's own
    ///   renderers consume direction-aware geometry, so only engines accumulating
    ///   advances in logical order hit it.
    ///
    /// Note for the fix: once engines place RTL clusters visually, their stored
    /// `ShapedCluster.x` is no longer monotonic in list order — consumers must not derive
    /// widths from origin differences (`next.x − first.x`); group widths are the sum of
    /// the group's cluster advances (`attribute_runs` already does — the width bug's
    /// regression test
    /// `attribute_runs_rtl_group_width_is_direction_independent` in
    /// `examples/shared/src/attributed_text.rs` is green).
    ///
    /// Ignored until the engines' RTL positioning is fixed (pre-existing, separate
    /// work item); run it manually with `cargo test -p massive-shapes --lib
    /// rtl_runs_position_clusters_visually -- --include-ignored --nocapture`.
    #[test]
    #[ignore = "pre-existing bug: both engines mirror RTL (see doc comment + probe_rtl_dump)"]
    fn rtl_runs_position_clusters_visually() {
        // The outer Arabic letters always shape into independent clusters (the middle
        // lam-alef may ligate). (text, Arabic byte starts): pure RTL paragraph vs RTL
        // run in an LTR paragraph — both against the bundled Amiri fixture, so the test
        // is hermetic (no system font database).
        let cases = [("سلام", vec![0, 2, 4, 6]), ("abسلام", vec![2, 4, 6, 8])];
        for (text, byte_starts) in cases {
            for kind in all_engines() {
                let mut fonts = FontManager::bare(kind).with_font(AMIRI);
                let request = ShapingRequest::new(text, TextAttributes::named_family("Amiri"));
                let run = {
                    let mut shaper = fonts.shaper();
                    shaper
                        .shape(&request, 16.0)
                        .expect("shaping must produce a run")
                };
                // Clusters keyed by first byte (logical position): the storage order
                // differs per engine (parley: logical, cosmic: visual).
                let mut by_byte: Vec<(usize, f32)> = run
                    .clusters
                    .iter()
                    .filter(|c| c.byte_range.start >= byte_starts[0])
                    .map(|c| (c.byte_range.start, c.x))
                    .collect();
                by_byte.sort_unstable_by_key(|(byte, _)| *byte);
                assert!(
                    by_byte.len() >= byte_starts.len() - 1,
                    "{kind:?} {text:?}: the Arabic letters must shape into multiple \
                     clusters (got {by_byte:?})",
                );
                assert!(
                    by_byte.windows(2).all(|w| w[0].1 > w[1].1),
                    "{kind:?} {text:?}: RTL clusters must sit right-to-left in logical \
                     order — Arabic renders mirrored (byte_start/x: {by_byte:?})",
                );
            }
        }
    }

    /// Per-range family overrides must shape the range with the overridden family on every
    /// engine. Parity probe: load two distinct fonts, shape `AB` with JetBrains Mono as
    /// the default family and `B` overridden to Amiri, then shape `B`'s text with Amiri as
    /// the *default* family; the overridden cluster must resolve to the same face as the
    /// explicit-default run. cosmic-text honors the override (`attrs_list.add_span`), but
    /// the Parley engine pushes only `StyleProperty::FontWeight` per range
    /// (`parley_engine::shape`), silently dropping `TextAttributes::family` — there `B`
    /// shapes with the *default* family's face instead.
    ///
    /// Two fonts are required: with a single loaded font, family fallback collapses both
    /// runs onto the same face and the probe is vacuous.
    #[test]
    fn range_family_override_shapes_like_explicit_default_family() {
        let text = "AB";
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind)
                .with_font(JETBRAINS_MONO)
                .with_font(AMIRI);
            // Overridden range: JetBrains Mono default, `B` (byte 1..2) forced to Amiri.
            let mut overridden =
                ShapingRequest::new(text, TextAttributes::named_family("JetBrains Mono"));
            overridden.ranges = vec![(1..2, TextAttributes::default().with_family("Amiri"))];
            // Reference: same text, Amiri as the family everywhere.
            let reference = ShapingRequest::new(text, TextAttributes::named_family("Amiri"));

            let run_overridden = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&overridden, 16.0)
                    .expect("shaping must produce a run")
            };
            let run_reference = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&reference, 16.0)
                    .expect("shaping must produce a run")
            };

            // Compare the `B` cluster's face and geometry: with the override honored, the
            // range shapes with the same face and origin as the all-Amiri run.
            fn cluster_for<'a, K: std::fmt::Debug>(
                kind: &K,
                run: &'a ShapedRun,
                byte: usize,
            ) -> &'a ShapedCluster {
                run.clusters
                    .iter()
                    .find(|c| c.byte_range.contains(&byte))
                    .unwrap_or_else(|| panic!("{kind:?}: no cluster covers byte {byte}"))
            }
            fn glyph_face(run: &ShapedRun, cluster: &ShapedCluster) -> FaceId {
                run.cluster_glyphs(cluster)[0].face_id
            }
            let b_overridden = cluster_for(&kind, &run_overridden, 1);
            let b_reference = cluster_for(&kind, &run_reference, 1);
            assert_eq!(
                glyph_face(&run_overridden, b_overridden),
                glyph_face(&run_reference, b_reference),
                "{kind:?}: range family override ignored — `B` shaped with a different face \
                 than the explicit-default-family run (family attributes silently dropped?)"
            );
            // The cluster's own advance is family-dependent (Amiri `B` vs fallback `B`
            // metrics) and independent of the preceding text — unlike the origin, which
            // legitimately shifts when the default-family `A` shapes differently.
            assert_eq!(
                b_overridden.advance, b_reference.advance,
                "{kind:?}: range family override ignored — `B` advances differently than \
                 in the explicit-default-family run"
            );
        }
    }

    /// After `load_font`, the manager must register the loaded font, whatever engine is behind
    /// it, so the returned ids are meaningful.
    #[test]
    fn load_font_registers_all_engine_faces() {
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            assert_eq!(fonts.engine_kind(), kind);
            let id = fonts.load_font(JETBRAINS_MONO)[0];
            assert!(
                fonts.shaper().font_data(id).is_some(),
                "{kind:?}: the loaded font must resolve to font data"
            );
        }
    }

    /// Shaping through the engine contract must produce glyphs whose `FaceId`s resolve via
    /// `font_data`, for every compiled-in engine. This is the fallback-safety invariant from the
    /// pre-engine registry design, restated engine-neutrally.
    #[test]
    fn shaped_faces_resolve_to_font_data() {
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            let request =
                ShapingRequest::new("a->b", TextAttributes::named_family("JetBrains Mono"));
            let run = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&request, 16.0)
                    .expect("shaping must produce a run")
            };
            assert!(!run.clusters.is_empty(), "{kind:?}: clusters must exist");
            let all_resolve = run.glyphs.iter().all(|g| {
                let mut shaper = fonts.shaper();
                shaper.font_data(g.face_id).is_some()
            });
            assert!(
                all_resolve,
                "{kind:?}: every shaped glyph's FaceId must resolve to font data"
            );
        }
    }

    /// Attribute identity echoed through shaping: shapes text with two adjacent attributed
    /// ranges carrying distinct metadata, with the mechanism enabled. Every cluster must echo
    /// the metadata of the range covering its first byte — including composed clusters
    /// straddling the boundary (base + combining mark shape into one cluster in both engines),
    /// which resolve to their first byte's range.
    ///
    /// The per-engine mechanisms differ; both must honor the same contract:
    /// - Parley resolves the cover per cluster (`engine::covering_metadata`).
    /// - cosmic-text propagates `Attrs::metadata` through shaping natively.
    #[test]
    fn metadata_echoes_first_byte_cover() {
        // (text, boundary byte offset — must fall on a grapheme edge, not inside a mark)
        for (text, boundary) in [("a->ba", 2), ("afiba", 3), ("a=+=b", 2), ("e\u{0301}ab", 3)] {
            for kind in all_engines() {
                let mut fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);

                let mut request = ShapingRequest::new(
                    text,
                    TextAttributes::named_family("JetBrains Mono").with_metadata(7),
                )
                .with_metadata();
                request.ranges = vec![
                    (
                        0..boundary,
                        TextAttributes::named_family("JetBrains Mono").with_metadata(1),
                    ),
                    (
                        boundary..text.len(),
                        TextAttributes::named_family("JetBrains Mono").with_metadata(2),
                    ),
                ];

                let run = {
                    let mut shaper = fonts.shaper();
                    shaper
                        .shape(&request, 16.0)
                        .expect("shaping must produce a run")
                };

                for cluster in &run.clusters {
                    let expected = crate::engine::covering_metadata(
                        &request.ranges,
                        request.default_attributes.metadata,
                        cluster.byte_range.start,
                    );
                    assert_eq!(
                        cluster.metadata,
                        expected,
                        "{kind:?} text {text:?}: cluster {:?} must echo its first byte's \
                         covering range (ranges {:?}, clusters {:?})",
                        cluster.byte_range,
                        request.ranges,
                        run.clusters
                            .iter()
                            .map(|c| (c.byte_range.clone(), c.metadata))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }

    /// The metadata mechanism is opt-in: without `with_metadata`, engines ignore metadata
    /// entirely and every shaped cluster reads `0` — even when ranges carry non-zero values.
    #[test]
    fn metadata_disabled_yields_zero_clusters() {
        let text = "abcd";
        for kind in all_engines() {
            let mut fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            let mut request =
                ShapingRequest::new(text, TextAttributes::named_family("JetBrains Mono"));
            request.ranges = vec![
                (
                    0..2,
                    TextAttributes::named_family("JetBrains Mono").with_metadata(1),
                ),
                (
                    2..4,
                    TextAttributes::named_family("JetBrains Mono").with_metadata(2),
                ),
            ];
            let run = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&request, 16.0)
                    .expect("shaping must produce a run")
            };
            assert!(
                run.clusters.iter().all(|c| c.metadata == 0),
                "{kind:?}: disabled metadata mechanism must leave clusters at 0"
            );
        }
    }

    /// No engine may emit a cluster with an *empty* `glyph_range`: consumers resolve the
    /// cluster's face through its first glyph (`cluster_glyphs(cluster)[0]`, e.g.
    /// `src/terminal/view.rs`), which panics on an empty slice.
    ///
    /// Probe battery: sequences that could plausibly shape to zero glyphs — stray combining
    /// marks and joiners without a base, zero-width characters, and lone tag/regional
    /// indicators — shaped against a monospace fixture that covers none of them, forcing
    /// `.notdef` / fallback paths on every engine.
    ///
    /// CONFIRMED, ignored until resolved (like the RTL probe): Parley really does emit
    /// the zero-glyph cluster — with the Jetbrains Mono fixture, `"a\u{200D}b"` shapes to
    /// a cluster covering bytes 1..4 (the lone ZWJ) with an empty `glyph_range`, so
    /// `cluster_glyphs(cluster)[0]` in `view.rs` panics on that input. The engine contract
    /// must gain "clusters are never empty" (engines skip zero-glyph clusters) or consumers
    /// must guard `is_empty`.
    ///
    /// Run manually: `cargo test -p massive-shapes --lib
    /// clusters_never_have_empty_glyph_ranges -- --include-ignored --nocapture`.
    #[test]
    #[ignore = "confirmed bug: Parley emits zero-glyph clusters (lone ZWJ); see doc comment"]
    fn clusters_never_have_empty_glyph_ranges() {
        let cases = [
            "\u{0301}",   // lone combining acute, no base
            "a\u{200D}b", // ZWJ between letters
            "\u{200D}",   // lone ZWJ
            "\u{200B}",   // zero-width space
            "\u{FEFF}",   // zero-width no-break space
            "a\u{FE0F}b", // variation selector-16 without an emoji base
            "\u{FE0F}",   // lone variation selector
            "\u{1F1E6}",  // lone regional indicator
        ];
        for text in cases {
            for kind in all_engines() {
                let mut fonts = FontManager::bare(kind)
                    .with_font(JETBRAINS_MONO)
                    .with_font(TAKRI);
                let request =
                    ShapingRequest::new(text, TextAttributes::named_family("JetBrains Mono"));
                let run = {
                    let mut shaper = fonts.shaper();
                    shaper
                        .shape(&request, 16.0)
                        .expect("shaping must produce a run")
                };
                for cluster in &run.clusters {
                    assert!(
                        !cluster.glyph_range.is_empty(),
                        "{kind:?} text {text:?}: cluster {:?} has zero glyphs — \
                         `cluster_glyphs(cluster)[0]` consumers would panic",
                        cluster.byte_range,
                    );
                }
            }
        }
    }
}
