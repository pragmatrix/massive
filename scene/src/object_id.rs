use std::{fmt, hash, marker::PhantomData};

use crate::{Change, Handle, Id, Object, Ref, SceneChange};

/// The id of an object, held in another object's value to refer to it.
///
/// It does not keep the object alive by itself. Once the referring object is submitted, the
/// lifetime stage counts the reference for as long as that object lives (ADR 0040). Until then,
/// the handle or reference the id was taken from has to keep the object alive.
pub struct ObjectId<T> {
    id: Id,
    object: PhantomData<fn() -> T>,
}

impl<T> ObjectId<T> {
    /// Wrap a raw id, for code that builds scene changes directly, such as the renderer's tests.
    /// Scene changes carry raw ids anyway, so this adds no way to refer to an unknown object.
    pub fn new(id: Id) -> Self {
        Self {
            id,
            object: PhantomData,
        }
    }

    pub fn id(self) -> Id {
        self.id
    }
}

impl<T: Object> From<&Handle<T>> for ObjectId<T>
where
    SceneChange: From<Change<T>>,
{
    fn from(handle: &Handle<T>) -> Self {
        Self {
            id: handle.id(),
            object: PhantomData,
        }
    }
}

impl<T: Object> From<&Ref<T>> for ObjectId<T>
where
    SceneChange: From<Change<T>>,
{
    fn from(reference: &Ref<T>) -> Self {
        Self {
            id: reference.id(),
            object: PhantomData,
        }
    }
}

// The traits are implemented by hand, because derives would require them of `T` as well.

impl<T> Clone for ObjectId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ObjectId<T> {}

impl<T> PartialEq for ObjectId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for ObjectId<T> {}

impl<T> hash::Hash for ObjectId<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T> fmt::Debug for ObjectId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ObjectId").field(&self.id).finish()
    }
}
