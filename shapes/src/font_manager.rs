//! The font manager: owns one shaping engine and exposes the stable shaping API.
//!
//! [`FontManager`] is the entry point consumers know (`system`/`bare`, `load_font`, `font_data`,
//! `shaper`). The concrete shaping work lives behind [`ShapingEngine`] implementations selected at
//! construction time (ADR 0005): the engine is chosen once at manager creation and every client
//! names it explicitly — there is no default engine — and a [`FaceId`] is only meaningful for the
//! engine instance that produced it.

use std::fmt;
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use crate::engine::{
    FontBytes, FontData, ShapedRun, ShapingEngine, ShapingEngineKind, ShapingRequest,
};
use crate::shaping_engines::{CosmicTextEngine, ParleyEngine};
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
}

impl fmt::Debug for FontManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock();
        f.debug_struct("FontManager")
            .field("engine", &inner.engine.name())
            .finish_non_exhaustive()
    }
}

struct FontManagerInner {
    /// Boxed: the engines carry large contexts (~1–2 kB: Parley's `FontContext` +
    /// `LayoutContext`, cosmic-text's `FontSystem`); an extra indirection per call is
    /// negligible next to shaping and keeps the manager handle small.
    engine: Box<dyn ShapingEngine>,
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
}

impl FontManager {
    /// Create a manager over the given engine kind, with system fonts loaded.
    pub fn system(kind: ShapingEngineKind) -> Self {
        let engine: Box<dyn ShapingEngine> = match kind {
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley => Box::new(ParleyEngine::system()),
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText => Box::new(CosmicTextEngine::system()),
        };
        Self {
            kind,
            inner: Arc::new(Mutex::new(FontManagerInner { engine })),
        }
    }

    /// A bare manager over the given engine kind: no fallbacks, no fonts.
    pub fn bare(kind: ShapingEngineKind) -> Self {
        let engine: Box<dyn ShapingEngine> = match kind {
            #[cfg(feature = "parley")]
            ShapingEngineKind::Parley => Box::new(ParleyEngine::bare()),
            #[cfg(feature = "cosmic-text")]
            ShapingEngineKind::CosmicText => Box::new(CosmicTextEngine::bare()),
        };
        Self {
            kind,
            inner: Arc::new(Mutex::new(FontManagerInner { engine })),
        }
    }

    /// Adds the font and returns Self
    pub fn with_font(self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Self {
        self.load_font(font_data);
        self
    }

    /// Adds the font and returns its font ids.
    pub fn load_font(&self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Vec<FaceId> {
        let mut inner = self.inner.lock();
        let data: FontBytes = Arc::new(font_data);
        inner.engine.load_font(data)
    }

    /// Resolve the concrete font data for a [`FaceId`] produced by this manager's engine.
    pub fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.inner.lock().engine.font_data(id)
    }

    /// The engine this manager shapes with.
    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.kind
    }

    /// Acquire a [`Shaper`], holding the manager's lock for the duration of the shaping session.
    #[must_use]
    pub fn shaper(&self) -> Shaper<'_> {
        Shaper {
            kind: self.kind,
            inner: self.inner.lock(),
        }
    }

    /// Shape a single line through the manager's engine.
    pub fn shape(&self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        self.inner.lock().engine.shape(request, font_size)
    }
}

impl Shaper<'_> {
    /// Shape one attributed line at `font_size` through the selected engine.
    pub fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        self.inner.engine.shape(request, font_size)
    }

    /// Resolve concrete font data through the engine of this session.
    ///
    /// Use this instead of [`FontManager::font_data`] while a shaper guard is held: the guard
    /// already owns the manager lock, so the manager method would self-deadlock.
    pub fn font_data(&mut self, id: FaceId) -> Option<FontData> {
        self.inner.engine.font_data(id)
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
    use crate::engine::TextAttributes;

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    fn all_engines() -> Vec<ShapingEngineKind> {
        ShapingEngineKind::available().to_vec()
    }

    /// After `load_font`, the manager must register the loaded font, whatever engine is behind
    /// it, so the returned ids are meaningful.
    #[test]
    fn load_font_registers_all_engine_faces() {
        for kind in all_engines() {
            let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            assert_eq!(fonts.engine_kind(), kind);
            let id = fonts.load_font(JETBRAINS_MONO)[0];
            assert!(
                fonts.font_data(id).is_some(),
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
            let fonts = FontManager::bare(kind).with_font(JETBRAINS_MONO);
            let request =
                ShapingRequest::new("a->b", TextAttributes::named_family("JetBrains Mono"));
            // The shaper guard holds the manager lock; drop it before `font_data`, which locks
            // the same mutex (the guard is not reentrant).
            let run = {
                let mut shaper = fonts.shaper();
                shaper
                    .shape(&request, 16.0)
                    .expect("shaping must produce a run")
            };
            assert!(!run.clusters.is_empty(), "{kind:?}: clusters must exist");
            let all_resolve = run
                .glyphs
                .iter()
                .all(|g| fonts.font_data(g.face_id).is_some());
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
}
