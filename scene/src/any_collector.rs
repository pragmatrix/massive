//! The task's change queue, erased so that one task-local can hold any collector type.
//!
//! A UI task collects exactly one queue of changes (ADR 0008). The queue's change type is fixed
//! at install time by [`AnyCollector::for_type`]; access through the collected sink is write-only,
//! typed access downcasts and panics loudly on a kind mismatch.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use massive_util::{ChangeCollector, ChangeSet};

use crate::{ChangeSink, SceneChange};

/// The change queue a UI task collects into.
///
/// Holds one collector per install, erased behind `Arc<dyn Any>` so its change type is invisible,
/// plus the sink cloned from the same allocation. There is no locking beyond the collector's own
/// mutex and no first-push type fixing: `for_type` erases the collector before any change exists.
pub struct AnyCollector {
    /// The sink `Handle<T>`s clone out at enter time. Erasure at the handle is what keeps one
    /// queue per task: a handle never sees a typed collector (ADR 0008).
    sink: Arc<dyn ChangeSink>,

    /// The typed view for writes that need the change type and for drains: the allocation
    /// `sink` was cloned from, erased.
    typed: Arc<dyn Any + Send + Sync>,
}

impl AnyCollector {
    /// Fix this queue's change type upfront, before any change is collected.
    pub fn for_type<C>() -> Self
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        let collector = Arc::new(ChangeCollector::<C>::default());
        Self {
            sink: collector.clone(),
            // Coerces to `Arc<dyn Any + Send + Sync>` in place: erasing the pointer _is_ the
            // erasure, so no second allocation wraps it.
            typed: collector,
        }
    }

    /// The erased sink handles push their changes into.
    pub fn sink(&self) -> &Arc<dyn ChangeSink> {
        &self.sink
    }

    /// Collect one change of the installed type.
    pub fn collect<C>(&self, change: impl Into<C>)
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        self.typed_collector::<C>().collect(change);
    }

    /// Drain all collected changes, preserving the type fixed at install.
    pub fn take_all<C>(&self) -> ChangeSet<C>
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        self.typed_collector::<C>().take_all()
    }

    /// The typed collector, downcasting to the type fixed at install.
    ///
    /// The erased `Arc` is not part of the downcast target: a `dyn Any` reports the pointee's type
    /// id, so the target is `ChangeCollector<C>` itself.
    fn typed_collector<C>(&self) -> &ChangeCollector<C>
    where
        C: From<SceneChange> + fmt::Debug + Send + 'static,
    {
        self.typed
            .downcast_ref::<ChangeCollector<C>>()
            .unwrap_or_else(|| panic_kind_mismatch::<C>(self.typed.as_ref().type_id()))
    }
}

impl fmt::Debug for AnyCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The erased value is not inspectable: report the queue kind, not the contents.
        f.debug_struct("AnyCollector")
            .field("sink", &self.sink)
            .finish_non_exhaustive()
    }
}

fn panic_kind_mismatch<C: 'static>(installed: TypeId) -> ! {
    panic!(
        "change collector was installed for another change type: asked for {:?}, installed type id {installed:?}",
        std::any::type_name::<C>()
    )
}

#[cfg(test)]
mod tests {
    use crate::{AnyCollector, Change, SceneChange};

    /// A second change type that embeds scene changes: the kind the wrong-drain test installs.
    /// The inner change is never read; the type identity is what the mismatch exercises.
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
    /// Every [`transform_change`] acquires a fresh id, so the drained ids themselves prove the
    /// order: the three pushes (sink, typed, sink) must come back as successive ids in exactly
    /// that sequence.
    #[test]
    fn erased_sink_and_typed_access_share_one_fifo() {
        let any = AnyCollector::for_type::<SceneChange>();

        any.sink().send(transform_change().into());
        any.collect::<SceneChange>(SceneChange::Transform(transform_change()));
        any.sink().send(transform_change().into());

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
        let any = AnyCollector::for_type::<InnerChange>();
        let _ = any.take_all::<SceneChange>();
    }

    /// One fresh transform update: every call acquires a new id, which is what the FIFO-order
    /// assertion reads back.
    fn transform_change() -> crate::Change<crate::Transform> {
        let id = crate::id_generator::acquire::<crate::Transform>();
        Change::Update(id, crate::Transform::IDENTITY)
    }
}
