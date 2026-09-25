# Instance runtime isolation plan

## Goal

Remove the cooperative-scheduling hack from the instance run loop and give each
instance a runtime that the OS scheduler, not tokio, preempts:

- instances no longer share the desktop's tokio worker pool;
- `MassiveTerminal::run` no longer needs `task::yield_now().await` per iteration; and
- the instance future's `Send` bound is no longer forced by spawning.

The change keeps the desktop application task, the render thread, and the shell
event-loop wiring untouched.

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
single-owner by design.

## Proposed direction

### 1. One dedicated thread and a `current_thread` runtime per instance

`InstanceManager::spawn` stops using `JoinSet::spawn`. Instead it spawns a
`std::thread` per instance whose closure builds a `current_thread` runtime,
blocks on `with_context(instance_task_context, AssertUnwindSafe(future).catch_unwind())`,
and reports `(InstanceId, result)` to the desktop over an mpsc channel that
`join_next()` now drains. The desktop's `select!` arms on `join_next` keep their
meaning; only the completion source changes.

Preemption moves to the OS: an instance can burn a full time slice and the
desktop thread still makes progress. The yield hack and its starvation comment
are deleted (mt `src/main.rs`).

`current_thread` covers every primitive the instance path uses: mpsc channels and
`Notify` are runtime-independent, `spawn_blocking` (the pty reader) runs on the
blocking pool, and everything owned by `TaskContext` lives inside the future's
own scope. `block_on` on the instance thread accepts `!Send` futures, dropping
the artificial constraint.

### 2. Mint `TaskContext` where the task-locals will live

The instance future must not read the desktop task's shaper task-local. The
shaping context is therefore minted on the desktop task — `with_shaper(|shaper|
shaper.new_context())` stays a desktop-task call — and passed into the thread
closure explicitly. Contexts are `Send + Sync` (ADR 0006), so the hand-off is
safe; the manager mutex is mint-time-only, so the thread never touches shared
shaping state. The remaining `TaskContext` parts (`AnyCollector`,
`AnimationCoordinator`, `MovementRuntime`) are constructed per instance without
ambient reads, as today.

The runtime is built inside the thread closure so its lifetime encloses
`block_on`: dropping a runtime before `spawn_blocking` work completes would
abort the pty reader.

### 3. Opt-in multi-thread runtime variant (deferred)

The wiring above is identical for both runtime kinds; only the builder lines
differ. Adding the option is a one-variant enum on the instance policy once real
parallelism inside an instance exists. No current instance has such a workload —
every instance frame is strictly sequential with the pty reader as its only
concurrent piece — so the variant waits for a first user.

### 4. Drop `mt`'s `#[tokio::main]`

`shell::run` already builds and enters a multi-thread runtime when none is
running (massive/shell/src/shell.rs). With instances off the shared pool that
runtime hosts only the application/desktop task, and `mt` can start without its
own runtime, removing one runtime layer and the `Handle::try_current` branch
from the launch path.

## Sequencing

1. Thread + `current_thread` isolation with `TaskContext` minting moved to the
   closure boundary; delete `yield_now` and the starvation comment.
2. Remove `#[tokio::main]` from mt.
3. The `InstanceRuntimeKind` enum (step 3 above) only when an instance needs
   internal parallelism.

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