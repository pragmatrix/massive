use massive_scene::{ChangedIds, Id, SceneChange, VisualRenderObj};

use crate::scene::{LocationTransforms, Scene};
use crate::{TransactionManager, Version};

/// The renderer's CPU-side copy of the scene: its contents, versions, pending changes, and the
/// location matrices derived from it.
///
/// It holds no GPU device, so updating it never touches the GPU and is safe without rendering,
/// for example while the window is occluded.
#[derive(Debug, Default)]
pub struct SceneState {
    transaction_manager: TransactionManager,
    scene: Scene,
    /// The changed visuals since the previous draw call.
    changed_visuals: ChangedIds,
    locations: LocationTransforms,
    /// The version the locations were last resolved at. Every applied change opens a new
    /// version, so while it is current, no location, transform, or visual changed and resolving
    /// again would recompute nothing.
    resolved_at: Version,
}

impl SceneState {
    /// Apply changes to the renderer hierarchy.
    ///
    /// This can be called multiple times with new changes without losing significant performance compared to combining all changes first. I.e. no expensive value computation is done here.
    pub fn apply_changes(&mut self, changes: impl IntoIterator<Item = SceneChange>) {
        let transaction = self.transaction_manager.new_transaction();

        for change in changes {
            if let SceneChange::Visual(visual_change) = &change {
                self.changed_visuals.add(visual_change.id());
            }
            self.scene.apply(change, &transaction);
        }
    }

    /// Drain the visuals that changed since the previous call, each with its current state, or
    /// `None` when it was deleted.
    pub fn take_changed_visuals(&mut self) -> impl Iterator<Item = (Id, Option<&VisualRenderObj>)> {
        let visuals = self.scene.visuals();
        self.changed_visuals
            .take_all()
            .map(move |id| (id, visuals[id].as_ref()))
    }

    /// Bring the matrices of the given locations up to date with the latest applied changes.
    ///
    /// Does not consume `location_ids` when nothing changed since the previous call.
    pub fn resolve_locations(&mut self, location_ids: impl Iterator<Item = Id>) {
        let head_version = self.transaction_manager.current();
        if self.resolved_at == head_version {
            return;
        }
        let transaction = self.transaction_manager.current_transaction();
        self.locations
            .resolve_locations_and_matrices(&self.scene, &transaction, location_ids);
        self.resolved_at = head_version;
    }

    /// The location matrices, valid for the locations passed to the latest
    /// [`Self::resolve_locations`].
    pub fn locations(&self) -> &LocationTransforms {
        &self.locations
    }
}
