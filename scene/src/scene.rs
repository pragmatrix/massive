use std::{
    any::TypeId,
    sync::{Arc, LazyLock},
};

use anyhow::Result;
use parking_lot::Mutex;

use crate::{
    Change, ChangeCollector, Handle, Object, SceneChange, type_id_generator::TypeIdGenerator,
};

/// A scene is the only direct connection of actual contents to the renderer. It tracks all the
/// changes to scene graph and uploads it when an update cycle ends.
///
/// A scene does not have direct observable changes, so it can always be shared and used for staging
/// objects onto it.
#[derive(Debug, Default)]
pub struct Scene {
    // This tracks all changes from staging, changing the the values in the handles, and dropping
    // them.
    //
    // Shared because handles need to push changes when dropped.
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
        let id = global_id_generator().lock().acquire(tid);
        Handle::new(id, value, self.change_tracker.clone())
    }

    // Take the changes that need to be sent to the renderer and release the ids in the process.
    pub fn take_changes(&self) -> Result<Vec<SceneChange>> {
        let changes = self.change_tracker.take_all();

        // Short circuit, to prevent locking the id generator.
        if changes.is_empty() {
            return Ok(Vec::new());
        }

        // Performance: May not lock the id generator if there are no destructive changes.
        let mut id_gen = global_id_generator().lock();

        // Free up all deleted ids (this is done immediately for now, but may be later done in the
        // renderer, for example to keep ids alive until animations are finished or cached resources
        // are cleaned up)
        for (type_id, id) in changes.iter().flat_map(|sc| sc.destructive_change()) {
            // Performance: Order by TypeId first to prevent expensive HashMap lookups?
            id_gen.release(type_id, id);
        }

        Ok(changes)
    }
}

/// ADR: Decided to use a global id generator, so that we can have multiple scenes per renderer.
fn global_id_generator() -> &'static Mutex<TypeIdGenerator> {
    static ID_GEN: LazyLock<Mutex<TypeIdGenerator>> =
        LazyLock::new(|| TypeIdGenerator::default().into());

    &ID_GEN
}
