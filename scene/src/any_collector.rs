//! The task's change queue, erased so that one task-local can hold any change type.
//!
//! A UI task collects exactly one queue of changes (ADR 0008). The queue's change type is fixed
//! at install time by [`AnyCollector::for_type`]. Typed access downcasts and panics on a kind
//! mismatch.

use std::any::{Any, TypeId};
use std::fmt;

use massive_util::ChangeSet;

use crate::SceneChange;

/// The change queue a UI task collects into, its change type fixed by [`AnyCollector::for_type`]
/// before any change exists (ADR 0008).
///
/// Only its own task touches the queue: handles push through the task-local, and the task's frame
/// submission drains it. So the queue is a plain value, without an `Arc` or a lock (ADR 0040).
pub struct AnyCollector {
    changes: Box<dyn ErasedChangeSet>,
}

impl AnyCollector {
    /// Fix this queue's change type upfront, before any change is collected.
    pub fn for_type<C>() -> Self
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        Self {
            changes: Box::new(ChangeSet::<C>::default()),
        }
    }

    /// Collect one change of the queue's change type.
    pub fn collect<C>(&mut self, change: impl Into<C>)
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        typed_changes::<C>(&mut *self.changes).push(change.into());
    }

    /// Collect one scene change, retyped into the queue's change type. This is how handles push.
    pub fn send(&mut self, change: SceneChange) {
        self.changes.push_scene_change(change);
    }

    /// Drain all collected changes, preserving the type fixed at install.
    pub fn take_all<C>(&mut self) -> ChangeSet<C>
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        std::mem::take(typed_changes::<C>(&mut *self.changes))
    }
}

impl fmt::Debug for AnyCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The erased value is not inspectable: report the queue, not the contents.
        f.debug_struct("AnyCollector").finish_non_exhaustive()
    }
}

/// A change set of any change type that scene changes convert into.
trait ErasedChangeSet: Send {
    fn push_scene_change(&mut self, change: SceneChange);

    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<C> ErasedChangeSet for ChangeSet<C>
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    fn push_scene_change(&mut self, change: SceneChange) {
        self.push(C::from(change));
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The downcast target is `ChangeSet<C>` itself: a `dyn Any` reports the pointee's type id, so the
/// erasing `Box` is not part of it.
fn typed_changes<C>(changes: &mut dyn ErasedChangeSet) -> &mut ChangeSet<C>
where
    C: From<SceneChange> + fmt::Debug + Send + 'static,
{
    let changes = changes.as_any_mut();
    let installed = (*changes).type_id();
    changes
        .downcast_mut::<ChangeSet<C>>()
        .unwrap_or_else(|| panic_kind_mismatch::<C>(installed))
}

fn panic_kind_mismatch<C: 'static>(installed: TypeId) -> ! {
    panic!(
        "change collector was installed for another change type: asked for {:?}, installed type id {installed:?}",
        std::any::type_name::<C>()
    )
}

#[cfg(test)]
mod tests {
    use crate::{AnyCollector, Change, Id, SceneChange, Transform};

    /// A second change type that embeds scene changes: the kind the wrong-drain test installs.
    #[derive(Debug)]
    struct InnerChange(#[allow(dead_code)] crate::Change<crate::Transform>);

    impl From<SceneChange> for InnerChange {
        fn from(change: SceneChange) -> Self {
            match change {
                SceneChange::Transform(inner) => Self(inner),
                other => unreachable!("test fixture only collects transforms: {other:?}"),
            }
        }
    }

    /// Q19: for_type + erased-sink pushes + typed access land in one FIFO, in call order.
    ///
    /// Every push acquires a fresh id, so the drained ids themselves prove the order.
    #[test]
    fn erased_sink_and_typed_access_share_one_fifo() {
        let mut any = AnyCollector::for_type::<SceneChange>();
        // The ids are recorded, because the id generator reuses freed ids, so their values say
        // nothing about the order they were acquired in.
        let pushed = [transform_id(), transform_id(), transform_id()];

        any.send(update(pushed[0]).into());
        any.collect::<SceneChange>(SceneChange::Transform(update(pushed[1])));
        any.send(update(pushed[2]).into());

        let drained: Vec<_> = any
            .take_all::<SceneChange>()
            .release()
            .into_iter()
            .map(|change| match change {
                SceneChange::Transform(Change::Update(id, _)) => id,
                other => unreachable!("test fixture only collects transform updates: {other:?}"),
            })
            .collect();
        assert_eq!(drained, pushed, "one FIFO received all pushes, in order");
    }

    /// Q19: a typed access with the wrong C panics naming the requested kind.
    #[test]
    #[should_panic(expected = "asked for \"massive_scene::change::SceneChange\"")]
    fn kind_mismatch_panics_with_requested_name() {
        let mut any = AnyCollector::for_type::<InnerChange>();
        let _ = any.take_all::<SceneChange>();
    }

    fn transform_id() -> Id<Transform> {
        crate::id_generator::acquire::<Transform>()
    }

    fn update(id: Id<Transform>) -> Change<Transform> {
        Change::Update(id, Transform::IDENTITY)
    }
}
