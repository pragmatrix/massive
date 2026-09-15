//! The cosmic-text per-handle shaping scratch (ADR 0006).

use crate::engine::{EngineScratch, FontData, FontRegistry, ShapedRun, ShapingRequest};
use crate::shaping_engines::cosmic_engine::CosmicTextEngine;
use crate::FaceId;

/// Cosmic-text shaping state for one manager handle: a shape-ready clone of the manager's
/// minted world (the *epoch-pull*, ADR 0006), seeded from the published snapshot.
///
/// [`EngineScratch::sync`] re-syncs whenever the published face count has moved — a loaded
/// font, or fallback faces interned by any other handle. The engine is built once and
/// updated incrementally (`CosmicTextEngine::pull`); a full rebuild would rescan the
/// registry per epoch move.
pub struct CosmicScratch {
    engine: Option<CosmicTextEngine>,
    /// Face count of the manager world at the last sync — the cosmic epoch token
    /// (a mismatch means faces were minted since and this scratch re-syncs).
    synced_faces: usize,
}

impl EngineScratch for CosmicScratch {
    fn sync(&mut self, published: &FontRegistry) {
        let face_count = published.face_count();
        if self.synced_faces != face_count {
            let Some(engine) = self.engine.as_mut() else {
                self.engine = Some(CosmicTextEngine::seed_from_registry(published));
                self.synced_faces = face_count;
                return;
            };
            engine.pull(published);
            self.synced_faces = face_count;
        }
    }

    fn shape(
        &mut self,
        request: &ShapingRequest<'_>,
        font_size: f32,
        mint_face: &mut dyn FnMut(FontData) -> Option<FaceId>,
    ) -> Option<ShapedRun> {
        let engine = self.engine.as_mut()?;
        // Unregistered fallback faces resolve through `mint_face` — the manager's canonical
        // engine interns the data under its lock and publishes at mint time, so every
        // minted `FaceId` is globally valid and its published snapshot carries the face.
        // This scratch's engine stays untouched; its next sync re-pulls from the snapshot
        // that now carries the face.
        let mint_face = &mut *mint_face;
        engine.shape_with_resolver(request, font_size, &mut move |seed, id, weight| {
            let data = seed.face_data(id)?;
            let _ = weight;
            mint_face(data)
        })
    }
}

impl CosmicScratch {
    pub(crate) fn new() -> Self {
        Self {
            engine: None,
            synced_faces: 0,
        }
    }
}