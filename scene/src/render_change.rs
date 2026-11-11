use derive_more::From;
use massive_geometry::Transform;

use crate::{Id, Location, Visual};

/// One structural change to a scene object as the renderer receives it.
///
/// Only the lifetime stage produces these, so a delete reaches the renderer only once nothing
/// refers to the object any more (ADR 0040).
#[derive(Debug, From)]
pub enum RenderSceneChange {
    Transform(RenderChange<Transform>),
    Location(RenderChange<Location>),
    Visual(RenderChange<Visual>),
}

/// A create, update, or delete of one object, keyed by its id.
#[derive(Debug)]
pub enum RenderChange<T> {
    Create(Id<T>, T),
    Update(Id<T>, T),
    Delete(Id<T>),
}

impl<T> RenderChange<T> {
    pub fn id(&self) -> Id<T> {
        match *self {
            RenderChange::Create(id, _)
            | RenderChange::Update(id, _)
            | RenderChange::Delete(id) => id,
        }
    }
}
