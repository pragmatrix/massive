# Instance runtime isolation plan

Decision record: docs/adr/0009-instance-runtimes-are-dedicated-threads.md.

## Goal

Remove the cooperative-scheduling hack from the instance run loop and give each
instance a runtime that the OS scheduler, not tokio, preempts:

- instances no longer share the desktop's tokio worker pool;
- `MassiveTerminal::run` no longer needs `task::yield_now().await` per iteration; and
- instances run on their own OS thread instead of the desktop task's worker pool.

The change keeps the desktop application task, the render thread, and the shell
event-loop wiring untouched. The instance future's `Send` bound deliberately
stays (see Current braid and ADR 0009).

## Current braid

All instance futures run on the shell's shared multi-thread runtime:
`InstanceManager::spawn` mints a `TaskContext` on the desktop task and pushes the
instance future into a `JoinSet` (massive/desktop/src/instance_manager.rs).

Every instance iteration is one long synchronous poll body — select, shape lines,
update the view, submit. Tokio cannot preempt synchronous work, so in debug builds
an instance under an output burst starves the desktop (the rationale recorded at
the `yield_now` call site in mt's `MassiveTerminal::run`). The yield fires
unconditionally and is the wrong granularity in both directions: pure overhead when
the desktop is otherwise idle, and not a real fix for a long synchronous body.

Spawning with `JoinSet` additionally forces the future to be `Send`, which pins
instance state to `Send`-only machinery even though each instance future is
single-owner by design. (Relaxing that bound is out of scope here: it would change
the public `Application` API for a benefit no current instance needs — recorded in
ADR 0009.)

## Proposed direction

### 1. One dedicated thread and a runtime per instance

`InstanceManager::spawn` stops using `JoinSet::spawn`. Instead it spawns a
`std::thread` per instance — named `instance <id>` for observability in samplers —
whose closure builds a tokio runtime, blocks on
`with_context(instance_task_context, AssertUnwindSafe(future).catch_unwind())`,
and reports `(InstanceId, result)` to the desktop over an mpsc channel that
`join_next()` now drains. The desktop's `select!` arms on `join_next` keep their
meaning; only the completion source changes.

The thread is detached: shutdown stays cooperative (`request_shutdown_all` plus
the bounded shutdown deadline); once the deadline expires the desktop bails and
the process end reaps any un-cooperative instance thread. The manager never joins
or aborts instance threads.

Per-application runtime flavor: each `Application` selects its runtime kind
(`current_thread` default, multi-thread opt-in) via a `RuntimeKind` field with a
builder setter. No current instance has the internal parallelism that would
warrant multi-thread, so the default stays cheap — but the choice lives at the
application-definition site from the start rather than as a later variant.

Preemption moves to the OS: an instance can burn a full time slice and the
desktop thread still makes progress. The yield hack and its starvation comment
are deleted (mt `src/main.rs`).

`current_thread` covers every primitive the instance path uses today: mpsc
channels and `Notify` are runtime-independent, and `spawn_blocking` (the pty
reader) runs on the blocking pool — the same pool any runtime flavor provides.
If an instance ever uses `tokio::spawn` or timers internally, that is the moment
its application opts into the multi-thread flavor.

### 2. Create `TaskContext` where the task-locals will live

The instance future must not read the desktop task's shaper task-local. The
shaping context is therefore created on the desktop task —
`task_context::fonts().new_shaping_context()` stays a desktop-task
call — and passed into the thread closure explicitly. `ShapingContext` is
`Send`, so the hand-off is safe; the manager mutex is registration-only, so the
thread never touches shared shaping state. The remaining `TaskContext` parts
(`AnyCollector`,
`AnimationCoordinator`, `MovementRuntime`) are constructed per instance without
ambient reads, as today.

The runtime is built inside the thread closure so its lifetime encloses
`block_on`: dropping a runtime before `spawn_blocking` work completes would
abort the pty reader.

### 3. Opt-in multi-thread runtime variant

Applied to the initial implementation (ADR 0009): the wiring is identical for both
runtime kinds — only the builder lines in the thread closure differ — so the
per-application choice costs nothing now. `current_thread` remains the default and
no instance opts into multi-thread today; adding it is one chained setter on
`Application`.

### 4. Drop `mt`'s `#[tokio::main]`

`shell::run` already builds and enters a multi-thread runtime when none is
running (massive/shell/src/shell.rs). With instances off the shared pool that
runtime hosts only the application/desktop task, and `mt` can start without its
own runtime, removing one runtime layer and the `Handle::try_current` branch
from the launch path.

## Sequencing

1. Thread + runtime isolation with `TaskContext` creation moved to the closure
   boundary, per-application `RuntimeKind`, named threads, detached-thread
   completion via mpsc; delete `yield_now` and the starvation comment.
2. Remove `#[tokio::main]` from mt.

## Validation

- `cargo check`/`cargo test` for massive-desktop, massive-applications, and mt
  (instance task-local tests must still pass inside the spawned thread).
- Debug-build burst reproducer: warm-cache rerun of `find .` in a terminal while
  confirming the desktop/launcher stays responsive — without the yield. This is
  the deciding experiment: if the desktop still stutters, something besides
  instance polling starves, and that must be identified before removing the
  hack.
- Smoke: instance teardown (close window, shutdown) still submits pending
  instances changes; the pty reading ends cleanly with the shell.