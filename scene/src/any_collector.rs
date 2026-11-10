//! The task's change queue, erased so that one task-local can hold any change type.
//!
//! A UI task collects exactly one queue of changes (ADR 0008). The queue's change type is fixed
//! at install time by [`AnyCollector::for_type`]. Typed access downcasts and panics on a kind
//! mismatch.

use std::any::{Any, TypeId};
use std::fmt;

use massive_util::ChangeSet;

use crate::{ChangeSink, SceneChange};

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

    /// Drain all collected changes, preserving the type fixed at install.
    pub fn take_all<C>(&mut self) -> ChangeSet<C>
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        std::mem::take(typed_changes::<C>(&mut *self.changes))
    }
}

impl ChangeSink for AnyCollector {
    fn send(&mut self, change: SceneChange) {
        self.changes.push_scene_change(change);
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
    use crate::{AnyCollector, Change, ChangeSink, SceneChange};

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

        any.send(transform_change().into());
        any.collect::<SceneChange>(SceneChange::Transform(transform_change()));
        any.send(transform_change().into());

        let drained = any.take_all::<SceneChange>().release();
        assert_eq!(drained.len(), 3, "one FIFO received all three pushes");

        let drained_ids: Vec<_> = drained
            .iter()
            .map(|change| match change {
                SceneChange::Transform(inner) => inner.id().to_usize(),
                other => unreachable!("test fixture only collects transforms: {other:?}"),
            })
            .collect();
        let mut sorted = drained_ids.clone();
        sorted.sort_unstable();
        assert_eq!(drained_ids, sorted, "FIFO order: ids come back ascending");

        assert!(
            drained
                .iter()
                .all(|change| matches!(change, SceneChange::Transform(_))),
            "every push arrived as the pushed kind"
        );
    }

    /// Q19: a typed access with the wrong C panics naming the requested kind.
    #[test]
    #[should_panic(expected = "asked for \"massive_scene::change::SceneChange\"")]
    fn kind_mismatch_panics_with_requested_name() {
        let mut any = AnyCollector::for_type::<InnerChange>();
        let _ = any.take_all::<SceneChange>();
    }

    fn transform_change() -> crate::Change<crate::Transform> {
        let id = crate::id_generator::acquire::<crate::Transform>();
        Change::Update(id, crate::Transform::IDENTITY)
    }
}
