//! How handles reach the change queue of the task they are used in (ADR 0040).
//!
//! `massive-scene` does not know how a task is identified. The layer that installs task contexts
//! registers a function that runs a closure with the current task's change sink, and every create,
//! update and release of a handle goes through it.

use std::sync::OnceLock;

use crate::{ChangeSink, SceneChange};

/// Runs a closure with the sink of the current task's change queue.
///
/// Returns `false` when no task context is installed, so the closure did not run.
pub type WithCurrentSink = fn(&mut dyn FnMut(&mut dyn ChangeSink)) -> bool;

static WITH_CURRENT_SINK: OnceLock<WithCurrentSink> = OnceLock::new();

/// Register how handles reach the current task's change queue.
///
/// The layer that installs task contexts registers it once per process, before it installs the
/// first one. A second registration panics, because one process identifies its tasks in only one
/// way.
pub fn register_current_queue(with_current_sink: WithCurrentSink) {
    assert!(
        WITH_CURRENT_SINK.set(with_current_sink).is_ok(),
        "The current queue is registered twice"
    );
}

/// Send one change into the current task's change queue.
pub(crate) fn send(change: SceneChange) {
    let mut change = Some(change);
    with_sink(&mut |sink| {
        sink.send(
            change
                .take()
                .expect("the current queue runs the closure once"),
        )
    });
}

/// Run `f` with the current task's sink.
///
/// Handles must be used inside a task (ADR 0040). Outside of one, the change is lost and its object
/// stays in the renderer. This does not panic in release builds, because a handle's drop can run
/// while a panic unwinds, and a second panic would abort the process.
fn with_sink(f: &mut dyn FnMut(&mut dyn ChangeSink)) {
    let reached = WITH_CURRENT_SINK
        .get()
        .is_some_and(|with_current_sink| with_current_sink(f));
    if !reached {
        log::error!("A scene handle was used outside a task context, so its change is lost");
        // A drop during a panic's unwinding would abort the process with a second panic.
        debug_assert!(
            reached || std::thread::panicking(),
            "A scene handle was used outside a task context (ADR 0040)"
        );
    }
}

/// A change queue for the tests of `massive-scene`, which sits below the layer that installs task
/// contexts. Each test thread gets its own queue, because tests run in parallel.
#[cfg(test)]
pub mod test_queue {
    use std::cell::RefCell;
    use std::sync::Once;

    use crate::{AnyCollector, ChangeSink, SceneChange};

    thread_local! {
        static QUEUE: RefCell<Option<AnyCollector>> = const { RefCell::new(None) };
    }

    /// Run `f` with a fresh change queue as the current task's queue.
    pub fn with_test_queue<R>(f: impl FnOnce() -> R) -> R {
        static REGISTERED: Once = Once::new();
        REGISTERED.call_once(|| super::register_current_queue(with_current_sink));
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

    fn with_current_sink(f: &mut dyn FnMut(&mut dyn ChangeSink)) -> bool {
        QUEUE.with_borrow_mut(|queue| queue.as_mut().map(|queue| f(queue)).is_some())
    }
}
