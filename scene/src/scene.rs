use std::sync::Arc;

use crate::{
    Change, ChangeCollector, Handle, Object, SceneChange, SceneChanges,
    id_generator::{self},
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
        let id = id_generator::acquire::<T>();
        Handle::new(id, value, self.change_tracker.clone())
    }

    /// Push external changes into this scene.
    pub fn push_changes(&self, changes: SceneChanges) {
        self.change_tracker.push_many(changes);
    }

    // Take the changes that need to be sent to the renderer.
    pub fn take_changes(&self) -> SceneChanges {
        self.change_tracker.take_all()
    }
}
