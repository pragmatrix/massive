# Movement runtime task-local context plan

## Goal

Make movement callbacks safe and ergonomic for task-facing applications, then
allow instances to finish their own animations during graceful close without
moving lifecycle ownership into the animation runtime.

The first change is the callback-local API. The per-instance settled drain is a
deferred follow-up; callback context must leave room for it without coupling
`massive-animation` to Tokio.

## Current situation and ownership

`MovementRuntime` owns mounted movements and invokes their apply, modifier, and
completion callbacks with explicit progress or allocator arguments
(`massive/animation/src/movement_runtime.rs`). `TaskMovementBuilder` adapts that
runtime for applications, while `AmbientAnimation` reaches the task's
`ANIMATION` state (`massive/applications/src/task_context/animation.rs`,
`massive/applications/src/ambient.rs`).

Movement callbacks execute while `ANIMATION` is already mutably borrowed. An
ambient call from a callback therefore re-enters the same `RefCell` and panics.
The task-facing `with_animation_and_movement` also exposes the runtime broadly,
so consumers can bypass a movement-specific handle. Allocation additionally
requires an open coordinator cycle; an ended movement's detached drain does not
start one.

The shell owns the application task context; each instance has its own
`TaskContext` and event receiver (`massive/shell/src/shell.rs`,
`massive/desktop/src/instance_manager.rs`). Instance teardown currently flushes
synchronously in `InstanceContext::drop`, which cannot wait for animations.
Shell-side ticks advance the shell runtime, not an ended instance's independent
runtime (`massive/applications/src/instance_context.rs`,
`massive/shell/src/application_context.rs`). Thus a drain must keep the instance
task alive and be coordinated at the shell/instance lifecycle boundary.

## Proposed direction

### Callback-local task API

Keep `massive-animation` generic and explicit; do not add Tokio there. Add a
private callback context, separate from `ANIMATION`, holding the active
callback's progress and, only while running a modifier, the scoped allocation
capability. A framework-neutral batch hook or wrapper in `MovementRuntime` may
be needed to let the task layer install that context once around each
`run_actions` sweep and each `apply_animations` sweep. Refresh its callback data
as each callback runs. Do not install task context separately for every queued
modifier: queued modifiers run under the runtime owner's task context, not the
task that enqueued them. The hook's exact name and signature remain
implementation design work.

`TaskMovementBuilder::mount` should return a task-specific movement handle.
Its `modify` callback receives only `&mut T`; callback-facing application code
retains the movement value but receives no separate progress or allocator
context. Apply, modifier, and completion callbacks all run in callback scope.
`AmbientAnimation::proceed()` reads that scope: apply/snap callbacks see the
exact `AnimationProgress`, while modifier and completion callbacks see
`Proceed` at the coordinator's current time. Outside callback scope, existing
frame-scoped ambient behavior remains unchanged.

Allocation is supported only inside modifier callbacks. It must update both the
shared `AnimationCoordinator` and that movement's `ending_time`. Ambient
allocation from apply or completion callbacks must panic with a clear
diagnostic. This includes `animate_if_changed` when the target change requires
allocation; its unchanged-target fast path may remain a no-op. Replace the broad
task-facing `with_animation_and_movement` escape hatch with narrower task
operations so applications cannot bypass the task-specific handle. Preserve
low-level runtime access needed by shell/application integration.

### Per-instance settled drain (deferred)

At the instance task/lifecycle boundary, a close or shutdown signal may first
initiate a fade-out. Keep that instance, its `TaskContext`, and its event
receiver alive while its own animation ticks continue until movement/animation
state is `Settled`; then submit final changes and terminate. This applies to
shutdown as well as an ordinary instance return. Renderer/desktop-only ticking
cannot advance an already-ended instance's independent runtime, so this is not
a `MovementRuntime` responsibility.

This per-instance drain is bounded by the one monotonic `Shutdown deadline`
that bounds graceful shutdown. It must not introduce an indefinite wait or a
separate per-instance timeout: once the global deadline expires, existing
forced termination semantics take precedence.

## Sequencing

1. Implement callback-local task API and runtime batch support: task-specific
   movement handles, callback scope for all callback kinds, progress routing,
   modifier-only allocation, and narrower task operations. Keep the low-level
   runtime reusable and framework-neutral.
2. Validate callback behavior and existing frame-scoped ambient behavior before
   considering lifecycle work.
3. Defer the instance settled drain. When taken up, implement it at the
   shell/instance lifecycle boundary, retaining the instance task context and
   event loop through final submission, and observe the existing global shutdown
   deadline. Do not put the drain in `MovementRuntime`.

## Validation

- Focused checks confirm apply/snap callbacks receive exact progress, modifier
  and completion callbacks receive `Proceed` at coordinator time, and
  `proceed()` outside callback scope retains frame-scoped behavior.
- Confirm ambient allocation succeeds only in a modifier callback, updates the
  coordinator and that movement's `ending_time`, and produces the documented
  diagnostic from apply/completion callbacks when allocation is required.
- Confirm queued modifiers use the runtime owner's callback context and that
  context installation occurs once per sweep, not once per modifier; verify
  task-facing users cannot reach the removed broad escape hatch while shell and
  application low-level integration still builds.
- For the deferred drain, verify close and shutdown keep an instance alive to
  `Settled`, submit final changes, and terminate; separately verify expiry of
  the global `Shutdown deadline` ends waiting even if an instance is unsettled.
