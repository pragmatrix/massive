use std::fmt;
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use crate::engine::{
    EngineScratch, FontData, FontRegistry, ShapedRun, ShapingEngineKind, ShapingRequest,
};
use crate::face_metrics::FaceMetrics;
use crate::{FaceId, GlyphRun};

use super::FontManager;
use super::state::FontManagerState;

/// A shaping owner with exclusive scratch and shared face identity state, for one task or
/// instance. Contexts from one [`crate::FontManager`] share the face authority, so they shape
/// concurrently (ADR 0006).
pub struct ShapingContext {
    state: Arc<FontManagerState>,
    scratch: Mutex<Box<dyn EngineScratch>>,
}

impl fmt::Debug for ShapingContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShapingContext")
            .field("engine", &self.state.published.kind)
            .finish_non_exhaustive()
    }
}

impl ShapingContext {
    pub(super) fn from_parts(
        state: Arc<FontManagerState>,
        scratch: Box<dyn EngineScratch>,
    ) -> Self {
        Self {
            state,
            scratch: Mutex::new(scratch),
        }
    }

    pub fn new_context(&self) -> Self {
        let scratch = self
            .state
            .authority
            .lock()
            .engine
            .new_scratch(&self.state.published.current.load_full());
        Self {
            state: Arc::clone(&self.state),
            scratch: Mutex::new(scratch),
        }
    }

    /// The font manager this context shapes for (ADR 0006).
    pub fn manager(&self) -> FontManager {
        FontManager::from_state(Arc::clone(&self.state))
    }

    /// Acquire a [`Shaper`] over this context's shaping state.
    ///
    /// Panics if this context already has a shaper open: the scratch mutex is non-reentrant, so
    /// exclusivity is runtime-enforced at the misuse point (ADR 0006).
    #[must_use]
    pub fn shaper(&self) -> Shaper<'_> {
        let mut scratch = self.scratch.try_lock().unwrap_or_else(|| {
            panic!(
                "ShapingContext shaper reentrancy: this context already has a shaper open (its \
                 scratch mutex is held); two live shapers on one context are unsupported"
            )
        });
        // Capture the snapshot before the sync so the session's metrics view stays a consistent
        // pre-open world; `shape` refreshes it from the manager's latest publication.
        let registry = self.state.published.current.load_full();
        scratch.sync(&registry);
        Shaper {
            kind: self.state.published.kind,
            context: self,
            scratch,
            registry,
        }
    }

    /// Publish the resolved face while the lock is held, so lock-free readers see it immediately.
    fn resolve_face(&self, data: FontData) -> Option<FaceId> {
        let mut authority = self.state.authority.lock();
        let id = authority.engine.resolve_face(data)?;
        self.state
            .published
            .current
            .store(authority.engine.font_registry());
        Some(id)
    }
}

/// A shaper over one [`ShapingContext`], shaping through that context's own scratch.
pub struct Shaper<'a> {
    kind: ShapingEngineKind,
    context: &'a ShapingContext,
    scratch: MutexGuard<'a, Box<dyn EngineScratch>>,
    /// Captured at session open and refreshed after each `shape`, so faces resolved during this
    /// session are visible to the frame's metrics and font-data reads.
    registry: Arc<FontRegistry>,
}

impl Shaper<'_> {
    pub fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        let context = self.context;
        let run = self
            .scratch
            .shape(request, font_size, &mut |data| context.resolve_face(data));
        self.registry = context.state.published.current.load_full();
        run
    }

    /// Resolve concrete font data through the session's registry snapshot.
    pub fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.registry.font_data(id)
    }

    /// The per-face metrics snapshot of this session.
    pub fn metrics(&self, id: FaceId) -> Option<&FaceMetrics> {
        self.registry.metrics(id)
    }

    pub fn registry(&self) -> &FontRegistry {
        &self.registry
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

    pub fn engine_kind(&self) -> ShapingEngineKind {
        self.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TextAttributes;
    use crate::{FontManager, ShapingRequest};

    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    fn all_engines() -> Vec<ShapingEngineKind> {
        ShapingEngineKind::available().to_vec()
    }

    #[test]
    fn contexts_from_one_manager_shape_concurrently() {
        for kind in all_engines() {
            let fonts = FontManager::bare(kind)
                .with_font(JETBRAINS_MONO)
                .expect("bundled font is valid");
            let first_context = fonts.new_shaping_context();
            let second_context = fonts.new_shaping_context();

            std::thread::scope(|scope| {
                let first = scope.spawn(move || {
                    let mut shaper = first_context.shaper();
                    shaper
                        .shape(
                            &ShapingRequest::new(
                                "first",
                                TextAttributes::named_family("JetBrains Mono"),
                            ),
                            16.0,
                        )
                        .expect("first context must shape")
                });
                let second = scope.spawn(move || {
                    let mut shaper = second_context.shaper();
                    shaper
                        .shape(
                            &ShapingRequest::new(
                                "second",
                                TextAttributes::named_family("JetBrains Mono"),
                            ),
                            16.0,
                        )
                        .expect("second context must shape")
                });

                assert!(
                    !first
                        .join()
                        .expect("first shaping thread must finish")
                        .glyphs
                        .is_empty()
                );
                assert!(
                    !second
                        .join()
                        .expect("second shaping thread must finish")
                        .glyphs
                        .is_empty()
                );
            });
        }
    }

    #[test]
    #[should_panic(expected = "ShapingContext shaper reentrancy")]
    fn one_context_rejects_reentrant_shaping() {
        let fonts = FontManager::bare(ShapingEngineKind::Parley)
            .with_font(JETBRAINS_MONO)
            .expect("bundled font is valid");
        let context = fonts.new_shaping_context();
        let _first = context.shaper();
        let _second = context.shaper();
    }

    /// Every glyph returned by a session must resolve through that same session's registry.
    #[test]
    fn shaped_faces_resolve_to_font_data() {
        for kind in all_engines() {
            let fonts = FontManager::bare(kind)
                .with_font(JETBRAINS_MONO)
                .expect("bundled font is valid");
            let request =
                ShapingRequest::new("a->b", TextAttributes::named_family("JetBrains Mono"));
            let context = fonts.new_shaping_context();
            let mut shaper = context.shaper();
            let run = shaper
                .shape(&request, 16.0)
                .expect("shaping must produce a run");
            assert!(!run.clusters.is_empty(), "{kind:?}: clusters must exist");
            let all_resolve = run.glyphs.iter().all(|g| {
                shaper.font_data(g.face_id).is_some() && shaper.metrics(g.face_id).is_some()
            });
            assert!(
                all_resolve,
                "{kind:?}: every shaped glyph's FaceId must resolve through the same session"
            );
        }
    }
}
