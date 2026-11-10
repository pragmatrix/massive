use derive_more::From;
use massive_geometry::Transform;

use crate::{Id, LocationRenderObj, VisualRenderObj};

/// One structural change to a scene object as the renderer receives it.
///
/// Only the lifetime stage produces these, so a delete reaches the renderer only once nothing
/// refers to the object any more (ADR 0040).
#[derive(Debug, From)]
pub enum RenderSceneChange {
    Transform(RenderChange<Transform>),
    Location(RenderChange<LocationRenderObj>),
    Visual(RenderChange<VisualRenderObj>),
}

/// A create, update, or delete of one object, keyed by its id.
#[derive(Debug)]
pub enum RenderChange<T> {
    Create(Id, T),
    Update(Id, T),
    Delete(Id),
}

impl<T> RenderChange<T> {
    pub fn id(&self) -> Id {
        match *self {
            RenderChange::Create(id, _)
            | RenderChange::Update(id, _)
            | RenderChange::Delete(id) => id,
        }
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_not_impl_any;

    use super::RenderSceneChange;

    /// Like a scene change, a render change owns its render object and is consumed by value, so
    /// it must not become `Clone` for convenience.
    #[test]
    fn render_scene_change_is_not_clone() {
        assert_not_impl_any!(RenderSceneChange: Clone);
    }
}
