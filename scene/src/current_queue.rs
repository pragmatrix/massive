//! How handles reach the change queue of the task they are used in (ADR 0040).
//!
//! `massive-scene` does not know how a task is identified. The layer that installs task contexts
//! registers a function that collects a change into the current task's queue, and every create,
//! update, retain and release of a handle goes through it.

use std::sync::OnceLock;
use std::thread;

use crate::SceneChange;

/// Collects a change into the current task's change queue.
///
/// Returns `false` when it could not reach the queue: no task context is installed, or the queue is
/// borrowed already, which happens when a handle drops while a panic unwinds out of a push.
pub type CollectChange = fn(SceneChange) -> bool;

static COLLECT_CHANGE: OnceLock<CollectChange> = OnceLock::new();

/// Register how handles reach the current task's change queue.
///
/// The layer that installs task contexts registers it once per process, before it installs the
/// first one. A second registration panics, because one process identifies its tasks in only one
/// way.
pub fn register_current_queue(collect_change: CollectChange) {
    assert!(
        COLLECT_CHANGE.set(collect_change).is_ok(),
        "The current queue is registered twice"
    );
}

/// Send one change into the current task's change queue.
///
/// Handles must be used inside a task (ADR 0040). Outside of one, the change is lost and its object
/// stays in the renderer. This does not panic in release builds, because a handle's drop can run
/// while a panic unwinds, and a second panic would abort the process.
pub fn send(change: SceneChange) {
    let reached = COLLECT_CHANGE
        .get()
        .is_some_and(|collect_change| collect_change(change));
    if !reached {
        log::error!(
            "A scene handle could not reach its task's change queue, so its change is lost"
        );
        // A drop during a panic's unwinding would abort the process with a second panic.
        debug_assert!(
            reached || thread::panicking(),
            "A scene handle could not reach its task's change queue (ADR 0040)"
        );
    }
}

/// A change queue for the tests of `massive-scene`, which sits below the layer that installs task
/// contexts. Each test thread gets its own queue, because tests run in parallel.
#[cfg(test)]
pub mod test_queue {
    use std::cell::RefCell;
    use std::sync::Once;

    use crate::{AnyCollector, SceneChange};

    thread_local! {
        static QUEUE: RefCell<Option<AnyCollector>> = const { RefCell::new(None) };
    }

    /// Run `f` with a fresh change queue as the current task's queue.
    pub fn with_test_queue<R>(f: impl FnOnce() -> R) -> R {
        static REGISTERED: Once = Once::new();
        REGISTERED.call_once(|| super::register_current_queue(collect_change));
        QUEUE.set(Some(AnyCollector::for_type::<SceneChange>()));
        let result = f();
        QUEUE.set(None);
        result
    }

    /// Drain the changes the current test queue collected so far.
    pub fn take_changes() -> Vec<SceneChange> {
        QUEUE.with_borrow_mut(|queue| {
            queue
                .as_mut()
                .expect("take_changes runs inside with_test_queue")
                .take_all::<SceneChange>()
                .release()
        })
    }

    fn collect_change(change: SceneChange) -> bool {
        QUEUE.with_borrow_mut(|queue| queue.as_mut().map(|queue| queue.send(change)).is_some())
    }
}
