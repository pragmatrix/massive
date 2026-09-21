//! The font manager: one font-identity authority plus per-context shaping sessions.
//!
//! [`FontManager`] owns the face-identity machinery (ADR 0006): the manager mutex covers
//! only font loading and session-path face resolution — the only work that must be serialized,
//! because it registers [`FaceId`]s and publishes the registry (with metrics). Shaping runs in
//! sessions over per-context scratch state: each [`ShapingContext`] holds its own shape-ready
//! scratch ([`EngineScratch`], created by its engine) and shapes without contending with other
//! contexts.
//!
//! ## Engine neutrality
//!
//! After construction the manager names no engine type: engines create their scratch via
//! [`ShapingEngine::new_scratch`], keep it registry-synced via [`EngineScratch::sync`], and
//! shape through [`EngineScratch::shape`]. Fonts loaded at any time are visible on the next
//! session: parley's shared collection self-syncs (fontique version sync), cosmic re-pulls
//! the published snapshot when its face count moved (registry sync). The one engine touch
//! point left here is the construction match in [`FontManager::system`] / [`FontManager::bare`].
//!
//! ## Shaper exclusivity without borrow-gating
//!
//! The registration entry point ([`FontManager::load_font`]) and context shaping entry point
//! ([`ShapingContext::shaper`]) take `&self`:
//! shaping never touches the manager mutex, so no compile-time borrow gate is needed.
//! Exclusivity is enforced at runtime instead: a shaper exclusively holds
//! its context's scratch mutex, and `shaper()` acquires it with `try_lock`, so two shapers
//! on one context panic loudly at the misuse point instead of deadlocking. Shapers on
//! different contexts shape in parallel — that is the point (ADR 0006); the manager mutex is
//! untouched by shaping.
//!
//! A shaper must not outlive the frame cycle it shaped for: `update_lines`-style call
//! sites hold one shaper per frame and drop it before anything the frame produced is
//! submitted — the ordering the renderer's freshness contract rests on.
//!
//! ## The published registry (with metrics)
//!
//! `FaceId` → font-data plus per-face swash [`FaceMetrics`] are published as an immutable
//! `Arc` snapshot ([`FontManager::published`]), read lock-free on the render and
//! lock-free on the render and glyph-placement paths. Every registration — `load_font`, and
//! face resolution from inside a shaper — republishes under the manager lock at registration
//! time, so a published snapshot never lags the known face world, and in-session resolution
//! ([`Shaper::font_data`]) is a plain lock-free map read.

use std::fmt;
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use super::font_policy::FontPolicy;
use super::shaping_context::ShapingContext;
use super::state::{FontAuthority, FontManagerState, PublishedRegistry};
use crate::FaceId;
use crate::engine::{FontRegistry, ShapingEngine, ShapingEngineKind};
#[cfg(feature = "cosmic-text")]
use crate::shaping_engines::CosmicTextEngine;
#[cfg(feature = "parley")]
use crate::shaping_engines::ParleyEngine;
/// A font manager sharing one canonical shaping engine and published registry.
///
/// Per-owner shaping scratch lives in [`ShapingContext`].
pub struct FontManager {
    state: Arc<FontManagerState>,
}

impl fmt::Debug for FontManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontManager")
            .field("engine", &self.state.published.kind)
            .finish_non_exhaustive()
    }
}

impl FontManager {
    fn with_engine(kind: ShapingEngineKind, engine: Box<dyn ShapingEngine>) -> Self {
        let published = Arc::new(PublishedRegistry {
            kind,
            current: ArcSwap::from(engine.font_registry()),
        });
        Self {
            state: Arc::new(FontManagerState {
                authority: Arc::new(Mutex::new(FontAuthority { engine })),
                published,
            }),
        }
    }

    /// Create a manager as `policy` prescribes.
    ///
    /// The policy is the whole construction input: which engine shapes, and whether system fonts
    /// are selectable. Each engine has one policy-driven constructor, so this is a plain dispatch
    /// over the compiled-in engines.
    pub fn new(policy: FontPolicy) -> Self {
        let kind = policy.engine();
        let engine: Box<dyn ShapingEngine> = match kind {
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley => Box::new(ParleyEngine::new(policy.system_fonts())),
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText => Box::new(CosmicTextEngine::new(policy.system_fonts())),
        };
        Self::with_engine(kind, engine)
    }

    /// A bare manager over the given engine kind: no fallbacks, no fonts.
    pub fn bare(kind: ShapingEngineKind) -> Self {
        Self::new(FontPolicy::new(kind, false))
    }

    /// Create a manager over the given engine kind, with system fonts loaded.
    pub fn system(kind: ShapingEngineKind) -> Self {
        Self::new(FontPolicy::new(kind, true))
    }

    /// Adds the font and returns Self
    pub fn with_font(self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Self {
        self.load_font(font_data);
        self
    }

    /// Adds the font and returns its font ids.
    ///
    /// Together with [`Self::shaping_context`], this is the *entire* entry surface for engine
    /// state. The manager mutex covers only this registration work — shapers shape per
    /// handle, lock-free (ADR 0006).
    ///
    /// Takes `&self`: shaping never holds the manager mutex (ADR 0006), so loading during
    /// an open shaper cannot deadlock — the runtime exclusivity guarantee lives on the
    /// shaper's scratch lock, not here.
    ///
    /// Font loading is possible at any time: parley sees the font through the shared
    /// collection; cosmic re-syncs on the next shaper's registry check.
    pub fn load_font(&self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Vec<FaceId> {
        self.load_fonts(std::iter::once(font_data))
            .pop()
            .expect("one font was loaded")
    }

    /// Adds multiple fonts and returns their font ids, publishing the registry once.
    pub fn load_fonts<T>(&self, font_data: impl IntoIterator<Item = T>) -> Vec<Vec<FaceId>>
    where
        T: AsRef<[u8]> + Sync + Send + 'static,
    {
        let mut authority = self.state.authority.lock();
        let ids = font_data
            .into_iter()
            .map(|data| authority.engine.load_font(Arc::new(data)))
            .collect();
        // The registry just mutated: republish while the lock is still held (see module doc).
        self.state
            .published
            .current
            .store(authority.engine.font_registry());
        ids
    }

    /// The engine this manager shapes with.
    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.state.published.kind
    }

    /// The last-published registry snapshot, lock-free.
    pub fn published(&self) -> Arc<FontRegistry> {
        self.state.published.current.load_full()
    }

    /// A render-only view of this manager's published registry.
    pub fn registry_source(&self) -> FontRegistrySource {
        FontRegistrySource {
            published: Arc::clone(&self.state.published),
        }
    }

    /// Create a fresh shaping owner with exclusive engine scratch.
    pub fn new_shaping_context(&self) -> ShapingContext {
        let scratch = self
            .state
            .authority
            .lock()
            .engine
            .new_scratch(&self.published());
        ShapingContext::from_parts(Arc::clone(&self.state), scratch)
    }

    /// Wrap existing manager state as a handle.
    ///
    /// Used by [`ShapingContext::manager`] to hand back the very manager a context shapes for:
    /// one face authority and one published registry are shared by the manager and all its
    /// contexts, so this is the same manager, not a second one.
    pub(crate) fn from_state(state: Arc<FontManagerState>) -> Self {
        Self { state }
    }
}

/// A render-only handle into a [`FontManager`]'s published registry (ADR 0006).
///
/// The renderer resolves glyphs lock-free through the latest-published snapshot and checks
/// runs' engine kind against the manager's (a debug assert). It never shapes — so it needs
/// neither the face authority nor a shaping scratch, which a full [`FontManager`] handle
/// would carry. Cheap to clone (two `Arc`s).
#[derive(Clone)]
pub struct FontRegistrySource {
    /// The manager's engine identity and published snapshot, swapped at every registration.
    published: Arc<PublishedRegistry>,
}

impl fmt::Debug for FontRegistrySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontRegistrySource")
            .field("engine", &self.published.kind)
            .finish_non_exhaustive()
    }
}

impl FontRegistrySource {
    /// The engine kind of the manager this source observes.
    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.published.kind
    }

    /// The last-published registry snapshot, lock-free (see [`FontManager::published`]).
    pub fn registry(&self) -> Arc<FontRegistry> {
        self.published.current.load_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{ShapedCluster, ShapedRun, ShapingRequest, TextAttributes};

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// A bundled font that exercises nonzero vertical glyph offsets, which the other
    /// bundled fixtures (monospace, Montserrat) lack entirely. See the sign test below
    /// for the fixture rationale. OFL 1.1, licensed alongside the font file.
    const TAKRI: &[u8] =
        include_bytes!("../../../assets/fonts/NotoSansTakri/NotoSansTakri-Regular.ttf");

    /// A bundled Arabic+Latin font so the bidi/RTL tests don't depend on the system
    /// font database (Arabic coverage is optional on many systems, and system fallback
    /// picks vary per platform). OFL 1.1, licensed alongside the font file.
    const AMIRI: &[u8] = include_bytes!("../../../assets/fonts/Amiri/Amiri-Regular.ttf");

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
            let fonts = FontManager::bare(kind).with_font(TAKRI);
            let request =
                ShapingRequest::new(text, TextAttributes::named_family("Noto Sans Takri"));
            let run = {
                let context = fonts.new_shaping_context();
                let mut shaper = context.shaper();
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
        // The offset path differs per engine (Parley scales in the font pipeline, cosmic-text
        // scales em-relative afterwards), so allow a small float tolerance here — but nothing
        // near the sign range (±3 px).
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
            let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            let request =
                ShapingRequest::new(descenders, TextAttributes::named_family("JetBrains Mono"));
            let run = {
                let context = fonts.new_shaping_context();
                let mut shaper = context.shaper();
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
                let fonts = FontManager::bare(kind).with_font(AMIRI);
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
                    let context = fonts.new_shaping_context();
                    let mut shaper = context.shaper();
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
                let fonts = FontManager::bare(kind).with_font(AMIRI);
                let request = ShapingRequest::new(text, TextAttributes::named_family("Amiri"));
                let run = {
                    let context = fonts.new_shaping_context();
                    let mut shaper = context.shaper();
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
            let fonts = FontManager::bare(kind)
                .with_font(JETBRAINS_MONO)
                .with_font(AMIRI);
            // Overridden range: JetBrains Mono default, `B` (byte 1..2) forced to Amiri.
            let mut overridden =
                ShapingRequest::new(text, TextAttributes::named_family("JetBrains Mono"));
            overridden.ranges = vec![(1..2, TextAttributes::default().with_family("Amiri"))];
            // Reference: same text, Amiri as the family everywhere.
            let reference = ShapingRequest::new(text, TextAttributes::named_family("Amiri"));

            let run_overridden = {
                let context = fonts.new_shaping_context();
                let mut shaper = context.shaper();
                shaper
                    .shape(&overridden, 16.0)
                    .expect("shaping must produce a run")
            };
            let run_reference = {
                let context = fonts.new_shaping_context();
                let mut shaper = context.shaper();
                shaper
                    .shape(&reference, 16.0)
                    .expect("shaping must produce a run")
            };

            // Compare the `B` cluster's face and geometry: with the override honored, the
            // range shapes with the same face and origin as the all-Amiri run.
            fn cluster_for<'a, K: fmt::Debug>(
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
            let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            assert_eq!(fonts.engine_kind(), kind);
            let id = fonts.load_font(JETBRAINS_MONO)[0];
            let context = fonts.new_shaping_context();
            assert!(
                context.shaper().font_data(id).is_some(),
                "{kind:?}: the loaded font must resolve to font data"
            );
        }
    }

    /// Batch loading returns every registration and publishes all resulting faces.
    #[test]
    fn load_fonts_registers_every_batch_item() {
        for kind in all_engines() {
            let fonts = FontManager::bare(kind);
            let batches = fonts.load_fonts([JETBRAINS_MONO, JETBRAINS_MONO]);

            assert_eq!(batches.len(), 2);
            for id in batches.into_iter().flatten() {
                assert!(fonts.published().font_data(id).is_some());
            }
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
                let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);

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
                    let context = fonts.new_shaping_context();
                    let mut shaper = context.shaper();
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
            let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
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
                let context = fonts.new_shaping_context();
                let mut shaper = context.shaper();
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
    /// Parley skips zero-glyph clusters, including a lone ZWJ in `"a\u{200D}b"`, preserving
    /// the engine contract that lets consumers safely resolve a cluster's first glyph.
    #[test]
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
                let fonts = FontManager::bare(kind)
                    .with_font(JETBRAINS_MONO)
                    .with_font(TAKRI);
                let request =
                    ShapingRequest::new(text, TextAttributes::named_family("JetBrains Mono"));
                let run = {
                    let context = fonts.new_shaping_context();
                    let mut shaper = context.shaper();
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
