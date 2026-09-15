//! The cosmic-text per-handle shaping scratch (ADR 0006).

use std::sync::Arc;

use crate::FaceId;
use crate::engine::{EngineScratch, FontData, FontRegistry, ShapedRun, ShapingRequest};
use crate::shaping_engines::cosmic_engine::CosmicTextEngine;

/// Cosmic-text shaping state for one manager handle: a shape-ready clone of the manager's
/// known face world (the cosmic *registry sync*, ADR 0006), seeded from the published
/// snapshot.
///
/// [`EngineScratch::sync`] re-syncs whenever the published face count has moved — a loaded
/// font, or fallback faces resolved by any other handle. The engine is built once and
/// updated incrementally (`CosmicTextEngine::pull`); a full rebuild would rescan the
/// registry per sync move.
pub struct CosmicScratch {
    engine: Option<CosmicTextEngine>,
    /// The canonical engine's prepared fallback candidate pool (empty for `bare()`),
    /// cloned into the seed — the catalog itself is scanned once, in `system()`.
    candidate_pool: Arc<fontdb::Database>,
    /// Face count of the manager world at the last sync — the cosmic registry-sync token
    /// (a mismatch means faces were resolved since and this scratch re-syncs).
    synced_faces: usize,
}

impl EngineScratch for CosmicScratch {
    fn sync(&mut self, published: &FontRegistry) {
        let face_count = published.face_count();
        if self.synced_faces != face_count {
            let Some(engine) = self.engine.as_mut() else {
                self.engine = Some(CosmicTextEngine::seed_from_registry(
                    published,
                    &self.candidate_pool,
                ));
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
        resolve_face: &mut dyn FnMut(FontData) -> Option<FaceId>,
    ) -> Option<ShapedRun> {
        let engine = self.engine.as_mut()?;
        // Unregistered fallback faces resolve through `resolve_face` — the manager's canonical
        // engine registers the data under its lock and publishes at registration time, so
        // every resolved `FaceId` is globally valid and its published snapshot carries the
        // face. This scratch's engine stays untouched; its next sync re-pulls from the
        // snapshot that now carries the face.
        let resolve_face = &mut *resolve_face;
        engine.shape_with_resolver(request, font_size, &mut move |seed, id, weight| {
            let data = seed.face_data(id)?;
            let _ = weight;
            resolve_face(data)
        })
    }
}

impl CosmicScratch {
    /// `candidate_pool` comes from the canonical engine's scan; a bare engine hands an
    /// empty one.
    pub(crate) fn new(candidate_pool: Arc<fontdb::Database>) -> Self {
        Self {
            engine: None,
            candidate_pool,
            synced_faces: 0,
        }
    }
}
