use std::any::TypeId;

use massive_geometry::Transform;

use crate::{
    Change, Id, Location, RenderChange, RenderSceneChange, SceneChange, Visual, id_generator,
};

/// Decides when scene objects end (ADR 0040).
///
/// The stage sits between the queues that collect scene changes and the renderer. It keeps a
/// reference count for each object, in which the object's handle counts as one reference. A create
/// starts the count at one, each retain adds a reference, and each release removes one. When the count reaches zero,
/// the stage emits the object's delete and returns its id to the id generator.
///
/// Each batch stays whole: an object released in a batch is deleted in that same batch. The renderer
/// relies on that, because it resolves references only between batches.
#[derive(Debug, Default)]
pub struct LifetimeStage {
    transforms: ReferenceCounts,
    locations: ReferenceCounts,
    visuals: ReferenceCounts,
}

impl LifetimeStage {
    /// Resolve one drained batch into the changes the renderer applies.
    pub fn resolve(&mut self, changes: Vec<SceneChange>) -> Vec<RenderSceneChange> {
        // Each change resolves into at most one render change, so the batch's length bounds the
        // result.
        let mut resolved = Vec::with_capacity(changes.len());
        let mut ended = Vec::new();

        for change in changes {
            let render_change = match change {
                SceneChange::Transform(change) => {
                    resolve_kind::<Transform, _>(&mut self.transforms, change, &mut ended)
                        .map(RenderSceneChange::from)
                }
                SceneChange::Location(change) => {
                    resolve_kind::<Location, _>(&mut self.locations, change, &mut ended)
                        .map(RenderSceneChange::from)
                }
                SceneChange::Visual(change) => {
                    resolve_kind::<Visual, _>(&mut self.visuals, change, &mut ended)
                        .map(RenderSceneChange::from)
                }
            };
            resolved.extend(render_change);
        }

        id_generator::release_all(&ended);
        resolved
    }
}

/// Resolve one change of the object kind `O`, and record the id of an object that ended.
///
/// `O` is the object type whose ids the id generator hands out, which is what an ended id is
/// returned under.
fn resolve_kind<O: 'static, T>(
    counts: &mut ReferenceCounts,
    change: Change<T>,
    ended: &mut Vec<(TypeId, Id)>,
) -> Option<RenderChange<T>> {
    let resolved = counts.resolve(change)?;
    if let RenderChange::Delete(id) = resolved {
        ended.push((TypeId::of::<O>(), id));
    }
    Some(resolved)
}

/// The reference count of each object of one kind, indexed by the object's id. Zero marks an id
/// without a live object.
#[derive(Debug, Default)]
struct ReferenceCounts(Vec<u32>);

impl ReferenceCounts {
    /// Count one change, and return what the renderer receives for it. A retain, and a release that
    /// leaves references behind, return nothing.
    fn resolve<T>(&mut self, change: Change<T>) -> Option<RenderChange<T>> {
        match change {
            Change::Create(id, value) => {
                let index = id.to_usize();
                if index >= self.0.len() {
                    self.0.resize(index + 1, 0);
                }
                assert_eq!(self.0[index], 0, "Created object {id:?} is already alive");
                self.0[index] = 1;
                Some(RenderChange::Create(id, value))
            }
            Change::Update(id, value) => {
                debug_assert!(self.is_alive(id), "Updated object {id:?} is not alive");
                Some(RenderChange::Update(id, value))
            }
            Change::Retain(id) => {
                assert!(self.is_alive(id), "Retained object {id:?} is not alive");
                self.0[id.to_usize()] += 1;
                None
            }
            Change::Release(id) => {
                assert!(self.is_alive(id), "Released object {id:?} is not alive");
                let count = &mut self.0[id.to_usize()];
                *count -= 1;
                (*count == 0).then_some(RenderChange::Delete(id))
            }
        }
    }

    fn is_alive(&self, id: Id) -> bool {
        self.0.get(id.to_usize()).is_some_and(|count| *count > 0)
    }
}

#[cfg(test)]
mod tests {
    use massive_geometry::Transform;

    use super::LifetimeStage;
    use crate::{Change, RenderChange, RenderSceneChange, SceneChange, id_generator};

    /// A reference keeps its object alive after the handle is dropped, and the object ends in the
    /// batch that releases its last reference (ADR 0040).
    #[test]
    fn an_object_ends_with_its_last_reference_not_its_handle() {
        let mut stage = LifetimeStage::default();
        let id = id_generator::acquire::<Transform>();

        let first = stage.resolve(vec![
            SceneChange::Transform(Change::Create(id, Transform::IDENTITY)),
            SceneChange::Transform(Change::Retain(id)),
            // The handle is dropped, the reference remains.
            SceneChange::Transform(Change::Release(id)),
        ]);
        assert!(matches!(
            first.as_slice(),
            [RenderSceneChange::Transform(RenderChange::Create(created, _))] if *created == id
        ));

        let last = stage.resolve(vec![SceneChange::Transform(Change::Release(id))]);
        assert!(matches!(
            last.as_slice(),
            [RenderSceneChange::Transform(RenderChange::Delete(deleted))] if *deleted == id
        ));
    }
}
