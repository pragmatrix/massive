//! The kinds of scene objects: their number, their indices, and what code that handles objects of
//! every kind together needs.
//!
//! The indices and the number of kinds are defined together here, so arrays with one entry per kind
//! stay consistent with them. Nothing here can be named outside the crate (ADR 0040).

use massive_geometry::Transform;

use crate::{
    Change, Id, Location, LocationParent, Object, RenderChange, RenderSceneChange, SceneChange,
    Visual,
};

/// Keeps [`Object`] implemented by the crate's own object types only. This module is private, so
/// no other crate can name it.
pub trait Sealed {}

/// The number of object kinds. Arrays with one entry per kind have this length and are indexed by
/// [`Object::INDEX`].
pub const OBJECT_KINDS: usize = 3;

impl Sealed for Transform {}

impl Object for Transform {
    const INDEX: usize = 0;

    fn references(&self) -> References {
        References::default()
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Transform(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Transform(change)
    }
}

impl Sealed for Location {}

impl Object for Location {
    const INDEX: usize = 1;

    fn references(&self) -> References {
        let parent = match self.parent {
            LocationParent::Location(parent) => Some(parent.into()),
            LocationParent::Root(_) => None,
        };
        References::new([Some(self.transform.into()), parent])
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Location(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Location(change)
    }
}

impl Sealed for Visual {}

impl Object for Visual {
    const INDEX: usize = 2;

    fn references(&self) -> References {
        References::new([Some(self.location.into()), None])
    }

    fn scene_change(change: Change<Self>) -> SceneChange {
        SceneChange::Visual(change)
    }

    fn render_change(change: RenderChange<Self>) -> RenderSceneChange {
        RenderSceneChange::Visual(change)
    }
}

/// The delete the renderer receives for an ended object of each kind, indexed by [`Object::INDEX`].
/// The order of the entries must match the indices above.
pub const RENDER_DELETES: [fn(u32) -> RenderSceneChange; OBJECT_KINDS] = [
    render_delete::<Transform>,
    render_delete::<Location>,
    render_delete::<Visual>,
];

fn render_delete<T: Object>(index: u32) -> RenderSceneChange {
    T::render_change(RenderChange::Delete(Id::from_index(index)))
}

/// The id of an object of any kind: the index of its id, tagged with the [`Object::INDEX`] of its
/// kind.
///
/// Code that handles objects of every kind together, such as the lifetime stage's worklist, holds
/// these instead of typed ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaggedId {
    pub kind: usize,
    pub index: u32,
}

impl<T: Object> From<Id<T>> for TaggedId {
    fn from(id: Id<T>) -> Self {
        Self {
            kind: T::INDEX,
            index: id.index(),
        }
    }
}

/// What an object refers to. Two at most: a location refers to its transform and, unless it is a
/// root, to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct References([Option<TaggedId>; 2]);

impl References {
    /// The references of one object, built by its [`Object::references`].
    pub fn new(references: [Option<TaggedId>; 2]) -> Self {
        Self(references)
    }

    pub fn iter(self) -> impl Iterator<Item = TaggedId> {
        self.0.into_iter().flatten()
    }
}
