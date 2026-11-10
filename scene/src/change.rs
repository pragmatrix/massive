use derive_more::From;
use massive_geometry::Transform;

use crate::{Id, Location, Visual};

/// One structural change to a scene object, retyped into the receiving queue's change kind.
///
/// Handles send these into their task's change queue. The lifetime stage turns them into
/// [`RenderSceneChange`](crate::RenderSceneChange)s for the renderer (ADR 0040).
#[derive(Debug, From)]
pub enum SceneChange {
    Transform(Change<Transform>),
    Location(Change<Location>),
    Visual(Change<Visual>),
}

/// A create, update, retain, or release of one object, keyed by its id.
///
/// Handles and references never send a delete. They release the object, and the lifetime stage
/// decides when it ends (ADR 0040).
#[derive(Debug)]
pub enum Change<T> {
    Create(Id<T>, T),
    Update(Id<T>, T),
    Retain(Id<T>),
    Release(Id<T>),
}

impl<T> Change<T> {
    pub fn id(&self) -> Id<T> {
        match *self {
            Change::Create(id, _)
            | Change::Update(id, _)
            | Change::Retain(id)
            | Change::Release(id) => id,
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
