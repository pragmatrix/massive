use std::sync::Arc;

use crate::id_generator;
use crate::{Change, Handle, HandleChangeReceiver, Object, SceneChange, SceneChangeSet};

/// A scene is an independent collector and instantiator of changes that are meant to send to the
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

    /// Push an external change into this scene.
    ///
    /// The change must occupy the same identity space as this scene.
    pub fn push_change(&self, change: SceneChange) {
        self.change_receiver.send(change);
    }

    // Take all the changes.
    pub fn take_changes(&self) -> SceneChangeSet {
        self.change_receiver.take_changes()
    }
}
