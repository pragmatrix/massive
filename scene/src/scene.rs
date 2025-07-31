use std::{
    any::TypeId,
    sync::{Arc, Mutex},
};

use anyhow::Result;

use crate::{
    type_id_generator::TypeIdGenerator, Change, ChangeCollector, Handle, Object, SceneChange,
};

/// A scene is the only direct connection of actual contents to the renderer. It tracks all the
/// changes to scene graph and uploads it when an update cycle ends.
///
/// A scene does not have direct observable changes, so it can always be shared and used for staging
/// objects onto it.
#[derive(Debug, Default)]
pub struct Scene {
    // Each type requires its own id generator to ensure that the generated ids are contiguous
    // within that type.
    //
    // Robustness: Id generation should probably be done somewhere else to enable multiple scenes?
    id_generator: Mutex<TypeIdGenerator>,
    // This tracks all changes from staging, changing the the values in the handles, and dropping them.
    //
    // Shared because the Handles need to push changes on drop.
    change_tracker: Arc<ChangeCollector>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Put an object on the stage.
    pub fn stage<T: Object + 'static>(&self, value: T) -> Handle<T>
    where
        SceneChange: From<Change<T::Change>>,
    {
        let tid = TypeId::of::<T>();
        // Architecture: Can't we put the TypeIdGenerator and the ChangeTracker together to remove
        // the two locks here (second one is in Handle::new)?
        let id = self.id_generator.lock().unwrap().acquire(tid);
        Handle::new(id, value, self.change_tracker.clone())
    }

    // Take the changes that need to be sent to the renderer and release the ids in the process.
    pub fn take_changes(&self) -> Result<Vec<SceneChange>> {
        let changes = self.change_tracker.take_all();

        // Short circute, to prevent locking the id generator.
        if changes.is_empty() {
            return Ok(Vec::new());
        }

        // Optimization: May not lock the id generator if there are no destructive changes.
        let mut id_gen = self.id_generator.lock().unwrap();

        // Free up all deleted ids (this is done immediately for now, but may be later done in the
        // renderer, for example to keep ids alive until animations are finished or cached resources
        // are cleaned up)
        for (type_id, id) in changes.iter().flat_map(|sc| sc.destructive_change()) {
            // TODO: order by TypeId first?
            id_gen.release(type_id, id);
        }

        Ok(changes)
    }
}
