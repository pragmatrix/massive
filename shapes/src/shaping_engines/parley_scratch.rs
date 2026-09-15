//! Parley's per-handle shaping scratch (ADR 0006).

use crate::engine::{EngineScratch, FontData, FontRegistry, ShapedRun, ShapingRequest};
use crate::FaceId;
use crate::shaping_engines::parley_engine::{self, ParleySessionContexts};

/// Parley shaping state for one manager handle: session contexts over the engine's shared
/// fontique collection.
///
/// Seeded once by the engine's `new_scratch` at first session open; [`EngineScratch::sync`]
/// is a no-op afterwards — the shared-mode collection self-syncs (fontique's version
/// counter), so fonts loaded at any time are visible to the next layout without re-seeding.
pub struct ParleyScratch {
    contexts: Option<ParleySessionContexts>,
}

impl EngineScratch for ParleyScratch {
    fn sync(&mut self, _published: &FontRegistry) {
        // Nothing to do: seeded once at creation; the shared collection makes later
        // registrations visible to every clone via fontique's internal version sync.
    }

    fn shape(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        _mint_face: &mut dyn FnMut(FontData) -> Option<FaceId>,
    ) -> Option<ShapedRun> {
        let contexts = self.contexts.as_mut()?;
        parley_engine::shape_line(
            &mut contexts.font_context,
            &mut contexts.layout_context,
            request,
            font_size,
        )
    }
}

impl ParleyScratch {
    /// A scratch over freshly cloned session contexts (the engine's shared world).
    pub(crate) fn new(contexts: ParleySessionContexts) -> Self {
        Self {
            contexts: Some(contexts),
        }
    }
}
