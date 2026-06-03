use std::sync::Arc;

use crate::id_generator;
use crate::{Change, Handle, HandleChangeReceiver, Object, SceneChange, SceneChangeSet};

/// A scene is an indepent collector and instantiator of changes that are meant to send to the
/// renderer in a later submission.
///
/// It is used primarily for instantiating new `Handle<T>` objects.
#[derive(Debug)]
pub struct Scene {
    // This tracks all changes from staging, changing the values in the handles, and dropping
    // them.
    //
    // Shared because handles need to push changes when dropped.
    change_receiver: Arc<dyn HandleChangeReceiver>,
}

impl Scene {
    pub fn new(change_receiver: Arc<dyn HandleChangeReceiver>) -> Self {
        Self { change_receiver }
    }

    /// Put an object on the stage.
    pub fn stage<T: Object + 'static>(&self, value: T) -> Handle<T>
    where
        SceneChange: From<Change<T::Change>>,
    {
        let id = id_generator::acquire::<T>();
        Handle::new(id, value, self.change_receiver.clone())
    }

    // Push external changes into this scene.
    //
    // This can be useful for combining changes from lower layers.
    //
    // Safety: The changes need to occupy the same identity space (i.e. use the same id generator).
    // pub fn push_changes(&self, changes: SceneChangeSet) {
    //     self.change_receiver.collect_many(changes);
    // }

    // Take all the changes.
    pub fn take_changes(&self) -> SceneChangeSet {
        self.change_receiver.take_changes()
    }
}
