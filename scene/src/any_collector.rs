//! The task's change queue, erased so that one task-local can hold any collector type.
//!
//! A UI task collects exactly one queue of changes (ADR 0008). The queue's change type is fixed
//! at install time by [`AnyCollector::for_type`]; the sink is write-only, typed access downcasts
//! and panics on a kind mismatch.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use massive_util::{ChangeCollector, ChangeSet};

use crate::{ChangeSink, SceneChange};

/// The change queue a UI task collects into, its change type fixed by [`AnyCollector::for_type`]
/// before any change exists (ADR 0008).
pub struct AnyCollector {
    sink: Arc<dyn ChangeSink>,
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
            // Coercing the pointer to `Arc<dyn Any + Send + Sync>` _is_ the erasure; no second
            // allocation wraps it.
            typed: collector,
        }
    }

    pub fn sink(&self) -> &Arc<dyn ChangeSink> {
        &self.sink
    }

    /// Collect one change of the queue's change type.
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

    /// The downcast target is `ChangeCollector<C>` itself: a `dyn Any` reports the pointee's type
    /// id, so the erased `Arc` is not part of it.
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

    fn transform_change() -> crate::Change<crate::Transform> {
        let id = crate::id_generator::acquire::<crate::Transform>();
        Change::Update(id, crate::Transform::IDENTITY)
    }
}
