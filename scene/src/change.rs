use std::any::TypeId;

use derive_more::From;
use massive_geometry::Transform;

use crate::{Id, Location, LocationRenderObj, Visual, VisualRenderObj};

/// One structural change to a scene object, retyped into the receiving queue's change kind.
#[derive(Debug, From)]
pub enum SceneChange {
    Transform(Change<Transform>),
    Location(Change<LocationRenderObj>),
    Visual(Change<VisualRenderObj>),
}

impl SceneChange {
    pub fn destructive_change(&self) -> Option<(TypeId, Id)> {
        match self {
            SceneChange::Transform(Change::Delete(id)) => Some((TypeId::of::<Transform>(), *id)),
            SceneChange::Visual(Change::Delete(id)) => Some((TypeId::of::<Visual>(), *id)),
            SceneChange::Location(Change::Delete(id)) => Some((TypeId::of::<Location>(), *id)),
            // ... match exhaustive.
            SceneChange::Transform(_) | SceneChange::Location(_) | SceneChange::Visual(_) => None,
        }
    }
}

/// A create, update, or delete of one object, keyed by its id.
#[derive(Debug)]
pub enum Change<T> {
    Create(Id, T),
    Update(Id, T),
    Delete(Id),
}

impl<T> Change<T> {
    pub fn id(&self) -> Id {
        match *self {
            Change::Create(id, _) | Change::Update(id, _) | Change::Delete(id) => id,
        }
    }
}

#[cfg(test)]
mod tests {
    use massive_geometry::Transform;
    use static_assertions::assert_not_impl_any;

    use super::{Change, SceneChange};

    /// A change owns its render object and is consumed by value; cloning it would duplicate
    /// payloads, so the enum must not become `Clone` again for convenience.
    #[test]
    fn scene_change_is_not_clone() {
        assert_not_impl_any!(SceneChange: Clone);
    }

    /// The same holds for one object's change. `Transform` is the payload to probe: a derived
    /// `Clone` carries a `T: Clone` bound, which only this payload satisfies, so it is the
    /// instantiation that a re-introduced `Clone` would newly make compile.
    #[test]
    fn change_is_not_clone() {
        assert_not_impl_any!(Change<Transform>: Clone);
    }
}
