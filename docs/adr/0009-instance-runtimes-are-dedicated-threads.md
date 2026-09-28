# Instance runtimes are dedicated threads, not shared-pool tasks

## Status: accepted

## Problem

All instance futures run on the shell's shared multi-thread runtime:
`InstanceManager::spawn` pushes each instance future into a `JoinSet` on the desktop
task. Every instance iteration is one long synchronous poll body — select, shape
lines, update the view, submit. Tokio cannot preempt synchronous work, so in debug
builds an instance under an output burst starves the desktop completely;
`mt` carried an unconditional `task::yield_now()` per iteration as a mitigation,
which is overhead when the desktop is idle and not a real fix for a long synchronous
body. Spawning with `JoinSet` additionally forces the instance future to be `Send`.

## Decision

Each instance gets a dedicated OS thread created by `InstanceManager::spawn`, named
`instance <id>`, running its own tokio runtime built inside the thread closure so
its lifetime encloses `block_on`. The thread blocks on the instance future and
reports `(InstanceId, result)` to the desktop over an mpsc channel that
`join_next()` drains; the desktop's `select!` arms keep their meaning, only the
completion source changes.

The instance future must not read the desktop task's task-locals, so its
`TaskContext` — including the shaping context — is created on the desktop task and
moved into the thread closure explicitly (`ShapingContext` is `Send`).

Each `Application` chooses its runtime flavor: `current_thread` by default, with an
opt-in multi-thread variant for instances that have internal parallelism. No current
instance has such a workload — every instance frame is strictly sequential with the
pty reader as its only concurrent piece — so the default stays cheap.

Instance threads are detached. Shutdown remains cooperative (`request_shutdown_all`
plus the bounded shutdown deadline); once the deadline expires, the desktop bails,
the process ends, and the OS reaps un-cooperative instance threads. The manager
never joins or aborts instance threads.

`mt` no longer has its own `#[tokio::main]`: `shell::run` builds and enters a
multi-thread runtime when none is running, and that runtime now hosts only the
application task.

## Considered options

- **Keep the `JoinSet` spawn with `yield_now`.** Rejected: the yield is unconditional
  and wrong-grained in both directions; tokio cannot preempt synchronous work.
- **`task::spawn_blocking` closure hosting an inner runtime.** Rejected: it leaves
  the instance thread on the desktop runtime's blocking pool — the shared machinery
  the isolation is meant to escape — and hides the runtime-drop-vs-blocking-tasks
  ordering inside `Drop` instead of visible sequencing. Its one advantage (direct
  `JoinHandle` completion without the mpsc channel) was not worth the coupling.
  Verified empirically that a nested `current_thread` runtime inside a
  `spawn_blocking` closure runs `block_on` with timers and cross-thread wakes; the
  option was ruled out on design, not feasibility.
- **Process per instance.** Rejected: instances are composed into one 3D visor scene
  in one window by design — cross-process would need a serialized per-frame
  scene-change protocol, process-resident `Ref` identity, and a face authority that
  crosses process boundaries. That is a different system.

Note on `Send`: the `+ Send` bound on the boxed instance future (`RunInstanceBox`)
predates the spawn mechanism and stays. Relaxing it would change the desktop's
public `Application` API for a benefit nothing currently needs; it is deferred until
an instance has genuinely `!Send` state.

Note on panics: `catch_unwind` around the instance future reports the panic as the
instance result in test profiles, where unwinding is forced; release and dev
profiles set `panic = "abort"`, so an instance panic in a shipped binary aborts the
process regardless.

## Consequences

- Preemption moves to the OS scheduler: an instance can burn a full time slice while
  the desktop thread keeps making progress, without any cooperative yield.
- Instance state no longer sits on the shared worker pool; each instance owns its
  runtime, coordinator, movement runtime, and shaping scratch.
- The runtime is built inside the thread closure because dropping a runtime before
  its `spawn_blocking` work completes would abort the pty reader; the closure order
  (build runtime → `block_on` → drop) must stay intact.
- A deadline-expired instance thread leaks until process exit; accepted because the
  bail is immediately followed by desktop end, and `request_shutdown_all` covers the
  cooperative path.
- The `yield_now` hack and its starvation comment in `mt` are deleted.