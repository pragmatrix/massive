use std::fmt;
use std::sync::Arc;

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
///
/// The context *is* the session: shaping takes `&mut self`, so a second live shape on one context
/// is a compile error. A batch of shapes holds the context and reuses its scratch and its registry
/// snapshot across the whole batch.
pub struct ShapingContext {
    state: Arc<FontManagerState>,
    scratch: Box<dyn EngineScratch>,
    /// The published registry this context resolves against, captured by [`ShapingContext::refresh`]
    /// and refreshed after every shape, so faces resolved during a shape are visible to the
    /// frame's metrics and font-data reads before the caller inspects the returned run.
    registry: Arc<FontRegistry>,
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
        let registry = state.published.current.load_full();
        Self {
            state,
            scratch,
            registry,
        }
    }

    /// The font manager this context shapes for (ADR 0006).
    pub fn manager(&self) -> FontManager {
        FontManager::from_state(Arc::clone(&self.state))
    }

    /// Bring this context up to date with the manager's published face world: sync the scratch, then
    /// capture the snapshot its `metrics`/`font_data` reads resolve against.
    ///
    /// The owning guard calls this when it opens a batch, so a read taken before the batch's first
    /// shape sees the current world; `shape` syncs again before every shape on its own.
    pub fn refresh(&mut self) {
        self.sync_scratch();
        self.registry = self.state.published.current.load_full();
    }

    /// Shape a run through this context's exclusive scratch.
    pub fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        // Sync against the latest publication before shaping: a scratch seeded from stale registry
        // state shapes to an empty run or panics inside the engine when a font was loaded but is
        // not yet in the scratch, so the sync must not lag a shape (ADR 0006).
        self.sync_scratch();
        let state = &self.state;
        let run = self
            .scratch
            .shape(request, font_size, &mut |data| resolve_face(state, data));
        // Faces resolved during the shape were published under the manager lock; capture them so
        // this context's reads see the run's own fallback faces.
        self.registry = state.published.current.load_full();
        run
    }

    /// Bring the scratch in line with the manager's published face world (ADR 0006).
    fn sync_scratch(&mut self) {
        let registry = self.state.published.current.load_full();
        self.scratch.sync(&registry);
    }

    /// Resolve concrete font data through this context's registry snapshot.
    pub fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.registry.font_data(id)
    }

    /// The per-face metrics snapshot of this context.
    pub fn metrics(&self, id: FaceId) -> Option<&FaceMetrics> {
        self.registry.metrics(id)
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
        self.state.published.kind
    }
}

/// Publish the resolved face while the lock is held, so lock-free readers see it immediately.
fn resolve_face(state: &FontManagerState, data: FontData) -> Option<FaceId> {
    let mut authority = state.authority.lock();
    let id = authority.engine.resolve_face(data)?;
    state
        .published
        .current
        .store(authority.engine.font_registry());
    Some(id)
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
            let mut first_context = fonts.new_shaping_context();
            let mut second_context = fonts.new_shaping_context();

            std::thread::scope(|scope| {
                let first = scope.spawn(move || {
                    first_context
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
                    second_context
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
    fn a_context_shapes_a_batch() {
        let fonts = FontManager::bare(ShapingEngineKind::Parley)
            .with_font(JETBRAINS_MONO)
            .expect("bundled font is valid");
        let mut context = fonts.new_shaping_context();
        let first = context.shape(
            &ShapingRequest::new("a", TextAttributes::named_family("JetBrains Mono")),
            16.0,
        );
        let second = context.shape(
            &ShapingRequest::new("bb", TextAttributes::named_family("JetBrains Mono")),
            16.0,
        );
        assert!(first.is_some_and(|run| !run.glyphs.is_empty()));
        assert!(second.is_some_and(|run| !run.glyphs.is_empty()));
    }

    /// Every glyph returned by a shape must resolve through the same context.
    #[test]
    fn shaped_faces_resolve_to_font_data() {
        for kind in all_engines() {
            let fonts = FontManager::bare(kind)
                .with_font(JETBRAINS_MONO)
                .expect("bundled font is valid");
            let request =
                ShapingRequest::new("a->b", TextAttributes::named_family("JetBrains Mono"));
            let mut context = fonts.new_shaping_context();
            let run = context
                .shape(&request, 16.0)
                .expect("shaping must produce a run");
            assert!(!run.clusters.is_empty(), "{kind:?}: clusters must exist");
            let all_resolve = run.glyphs.iter().all(|g| {
                context.font_data(g.face_id).is_some() && context.metrics(g.face_id).is_some()
            });
            assert!(
                all_resolve,
                "{kind:?}: every shaped glyph's FaceId must resolve through the same context"
            );
        }
    }

    // The scratch is `Box<dyn EngineScratch>` and `EngineScratch: Send` without `Sync`, so a
    // context is movable but not shareable: two threads must not shape through one context.
    static_assertions::assert_impl_all!(ShapingContext: Send);
    static_assertions::assert_not_impl_any!(ShapingContext: Sync);
}
