use anyhow::Result;

use massive_geometry::PixelCamera;
use massive_scene::SceneChanges;

use crate::{RenderPacing, RenderTarget};

/// Rationale: This was introduced at a time camera updates were separate from scene changes,
/// leading to synchronization issues (or multiple redraws that were not needed).
///
/// So the intent here is to gather everything the renderer needs to know for an update and process
/// it all at once.
#[must_use]
#[derive(Debug, Default)]
pub struct RenderSubmission {
    pub changes: SceneChanges,
    pub pacing: RenderPacing,
    pub camera_update: Option<PixelCamera>,
}

impl RenderSubmission {
    pub fn new(changes: SceneChanges, pacing: RenderPacing) -> Self {
        Self {
            changes,
            pacing,
            camera_update: None,
        }
    }

    pub fn with_camera(mut self, camera: PixelCamera) -> Self {
        self.camera_update = Some(camera);
        self
    }

    pub fn with_pacing(mut self, pacing: RenderPacing) -> Self {
        self.pacing = pacing;
        self
    }

    pub fn submit_to(self, target: &mut dyn RenderTarget) -> Result<()> {
        target.render(self)
    }
}
