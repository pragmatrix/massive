use std::{fmt, marker::PhantomData};

use crate::lifetime_stage::{Kind, Targets};
use crate::{
    Change, Id, RenderChange, RenderSceneChange, SceneChange, current_queue, id_generator,
};

/// The single owner and writer of an object submitted to a queue.
///
/// Its changes go to the change queue of the task it is used in, so it must be used inside a task
/// (ADR 0040). A handle is not `Clone`, because a clone would hold its own copy of the value, and
/// two writers would overwrite each other's snapshots. Code that only keeps the object alive holds
/// a [`Ref`].
#[derive(Debug)]
pub struct Handle<T: Object> {
    id: Id<T>,
    // Optimization: Some values might be too large to be duplicated between the application and the
    // renderer.
    value: T,
}

impl<T: Object> Handle<T> {
    pub(crate) fn new(value: T) -> Self {
        let id = id_generator::acquire::<T>();
        current_queue::send(T::scene_change(Change::Create(id, value.clone())));
        Self { id, value }
    }

    pub fn id(&self) -> Id<T> {
        self.id
    }

    /// A counted reference that keeps the object alive, also after this handle is dropped.
    pub fn to_ref(&self) -> Ref<T> {
        Ref::retain(self.id)
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    /// Update the value of the handle.
    pub fn update(&mut self, value: T) {
        self.value = value;
        self.send_update();
    }

    pub fn update_if_changed(&mut self, value: T)
    where
        T: PartialEq,
    {
        if self.value != value {
            self.update(value);
        }
    }

    pub fn update_with(&mut self, f: impl FnOnce(&mut T)) {
        f(&mut self.value);
        self.send_update();
    }

    /// Set one field of the value, and send the update only when the field changes.
    ///
    /// `field` selects the field, so its current value and `value` can be compared directly.
    pub(crate) fn set_if_changed<F: PartialEq>(
        &mut self,
        field: impl FnOnce(&mut T) -> &mut F,
        value: F,
    ) {
        let slot = field(&mut self.value);
        if *slot == value {
            return;
        }
        *slot = value;
        self.send_update();
    }

    fn send_update(&self) {
        current_queue::send(T::scene_change(Change::Update(self.id, self.value.clone())));
    }
}

impl<T: Object> Drop for Handle<T> {
    fn drop(&mut self) {
        current_queue::send(T::scene_change(Change::Release(self.id)));
    }
}

/// A counted reference that keeps an object alive.
///
/// Each reference counts once in the lifetime stage, which deletes the object only after its handle,
/// every reference, and every live object that refers to it are gone (ADR 0040). A reference
/// carries only the object's id, so it cannot read the value.
pub struct Ref<T: Object> {
    id: Id<T>,
    object: PhantomData<fn() -> T>,
}

impl<T: Object> Ref<T> {
    pub fn id(&self) -> Id<T> {
        self.id
    }

    fn retain(id: Id<T>) -> Self {
        current_queue::send(T::scene_change(Change::Retain(id)));
        Self {
            id,
            object: PhantomData,
        }
    }
}

impl<T: Object> fmt::Debug for Ref<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Ref").field("id", &self.id).finish()
    }
}

impl<T: Object> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Self::retain(self.id)
    }
}

impl<T: Object> Drop for Ref<T> {
    fn drop(&mut self) {
        current_queue::send(T::scene_change(Change::Release(self.id)));
    }
}

/// A kind of scene object: a transform, a location, or a visual.
///
/// The renderer receives the value itself with every change, which is why a value refers to other
/// objects by [`Id`], never by a counted reference (ADR 0040). The trait is sealed, because the
/// lifetime stage and the renderer handle exactly these three kinds.
pub trait Object: Sized + Clone + fmt::Debug + sealed::Sealed {
    /// The kind, which selects the object's id space and its tables.
    const KIND: Kind;

    /// The objects this value refers to. The lifetime stage counts them for as long as the object
    /// lives.
    fn targets(&self) -> Targets;

    fn scene_change(change: Change<Self>) -> SceneChange;

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange;
}

/// Keeps [`Object`] implemented by the crate's own object types only.
pub mod sealed {
    pub trait Sealed {}
}
