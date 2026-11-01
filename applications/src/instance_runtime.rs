//! The run of one instance on its own runtime, which every host performs on the instance's
//! dedicated thread (ADR 0009).

use std::panic::AssertUnwindSafe;

use anyhow::{Context, Result, anyhow};
use futures::FutureExt;
use panic_message::panic_message as panic_payload_message;
use tokio::runtime::Builder;

use crate::InstanceFuture;
use crate::task_context::{self, TaskContext};

/// Build the instance runtime from `runtime` and drive the instance future to completion under
/// the instance's task context, converting a panic into an error result.
///
/// The host calls this on the thread it spawned for the instance and passes the builder of the
/// application's [`RuntimeKind`](crate::RuntimeKind), adding thread settings of its own, such as
/// the thread priority.
pub fn run_instance(
    mut runtime: Builder,
    instance: InstanceFuture,
    task_context: TaskContext,
) -> Result<()> {
    // The runtime is built inside the thread so its lifetime encloses `block_on`: dropping a
    // runtime before `spawn_blocking` work completes would abort the pty reader.
    let runtime = runtime
        .enable_all()
        .build()
        .context("building an instance runtime")?;
    let result = runtime.block_on(task_context::with_context(
        task_context,
        AssertUnwindSafe(instance).catch_unwind(),
    ));
    match result {
        Ok(result) => result,
        // `catch_unwind` yields `Box<dyn Any + Send>`, exactly the payload type
        // `panic_message` takes — it unpacks `&str`/`String` payloads like rustc's
        // default handler and falls back to "Box<dyn Any>" for anything else.
        Err(payload) => Err(anyhow!(
            "Instance panicked : {}",
            panic_payload_message(&payload)
        )),
    }
}
