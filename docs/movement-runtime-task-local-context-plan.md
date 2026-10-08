# Movement runtime task-local context plan

Status: **draft, not approved**. This document supersedes and merges the former
`movement-runtime-callback-scope-plan.md`; the callback-scope mechanism below is
part of this plan, not a separate one.

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

Re-entry is structural, not incidental. Every ambient accessor goes through
`with_animation_state`, which takes one `RefCell::borrow_mut` on the `ANIMATION`
task-local for the whole call. Movement callbacks run inside that borrow
(`AnimationState::flush_and_end_cycle` calls
`movement.run_actions(&mut self.coordinator)` from within it), so an ambient call
from a callback panics on re-entry, before any `FrameWitness` gate is consulted.
A callback context therefore cannot live in `ANIMATION`, and cannot be read
through `with_animation_state` either: it must own the data the runtime hands it.

`Movement<T>` is a mailbox, not a dispatcher. It only pushes into
`actions_inbox`; the callbacks run in `MovementRuntime::run_actions` and
`apply_animations`, on the runtime owner's task. The handle carries no task
relation and must not be where a callback context is installed. The task-facing
`with_animation_and_movement` also exposes the runtime broadly, so consumers can
bypass a movement-specific handle.

A queued modifier can allocate with no open cycle. `InstanceContext::drop` calls
`end_frame_cycle_detached`, which reaches `flush_and_end_cycle` and hence
`run_actions`, against a coordinator whose `cycle` is `None`; the allocator then
hits `AnimationCoordinator::cycle()`'s `expect("animation cycle must be started
before it is used")`. That panic happens inside a `Drop` impl, and `panic =
"abort"` is set for both `dev` and `release`, so it aborts the process. This is
reachable today through the explicit allocator and becomes a default-path problem
once ambient allocation is the ordinary way to start an animation.

The shell owns the application task context; each instance has its own
`TaskContext` and event receiver (`massive/shell/src/shell.rs`,
`massive/desktop/src/instance_manager.rs`). Instance teardown currently flushes
synchronously in `InstanceContext::drop`, which cannot wait for animations.
Shell-side ticks advance the shell runtime, not an ended instance's independent
runtime (`massive/applications/src/instance_context.rs`,
`massive/shell/src/application_context.rs`). Thus a drain must keep the instance
task alive and be coordinated at the shell/instance lifecycle boundary.

## Proposed direction

### Callback scope

Keep `massive-animation` generic and explicit; do not add Tokio there. Three
settled decisions fix the shape:

1. `Movement<T>` is a mailbox (see above), so the callback context must not be
   installed there.
2. The callback scope is installed **once per runtime sweep** — once per
   `run_actions` and once per `apply_animations` — never once per callback, and
   callbacks are never individually wrapped. Queued modifiers run under the
   runtime owner's task context, not the task that enqueued them.
3. The hook therefore belongs on the two sweep entry points in
   `MovementRuntime`, which is also the only place that knows each callback's
   kind, progress, and per-movement allocator.

#### Sweep hook in `massive-animation`

The runtime gains one neutral parameter per sweep and announces each callback
immediately before running it:

```rust
pub trait CallbackScope {
    fn begin_callback(&self, callback: MovementCallback<'_>);
}

pub enum MovementCallback<'a> {
    Apply(AnimationProgress),
    Modifier { instant: Instant, allocator: &'a mut dyn AnimationAllocator },
    Completion(Instant),
}

impl MovementRuntime {
    pub fn run_actions(&mut self, context: &mut dyn AnimationAllocator, scope: &dyn CallbackScope);
    pub fn apply_animations(&mut self, instant: Instant, scope: &dyn CallbackScope)
        -> Vec<Box<dyn Any + Send>>;
}
```

`begin_callback` is a field write on an already-installed scope, not an install.
`&dyn` with interior mutability rather than `&mut`, so the task-local can hold
the same scope the runtime is driving without aliasing it. Announce sites: the
`Modify` arm (with the `MovementAnimationAllocator` the runtime already builds)
and the `Snap` arm of `run_actions`; before `apply_animations` and before
`completion_event()` inside `apply_animations`. `Drop` announces nothing.
`Movement`, `MovementInstance`, `AnimationProgress`, `AnimationAllocator`,
`AnimationCoordinator`, and `CycleEnd` are unchanged, and no ambient or Tokio
vocabulary enters the animation crate.

#### One install per sweep in the task layer

Both sweeps are wrapped by the task layer, with the scope installed for the
sweep's duration: `AnimationState::flush_and_end_cycle` wraps `run_actions` +
`end_cycle`, and a new `task_context::apply_movement_animations` wraps
`apply_animations` for the shell. `tokio::task::LocalKey::sync_scope` is the
install primitive, verified in tokio 1.53.1 (the version this workspace
resolves) to accept a non-`'static` value and to swap the previous slot back on
exit, so one install covers a whole sweep and nested scopes still work.
`scoped-tls` is already in the lock file transitively, so a scoped-pointer slot
needs no new dependency; no precedent for either exists in first-party code
today. Ambient accessors then read the callback scope by pointer and never touch
`ANIMATION`, which is what makes them work from inside a callback at all.

#### Routing and rules

| Callback | `proceed()` returns | ambient `animate` / `animate_if_changed` |
|---|---|---|
| apply | the exact `AnimationProgress` (so `Snap` is the finish path) | panics if it would allocate; unchanged target is a silent no-op |
| modifier | `Proceed` at the coordinator's cycle time | allowed; updates the coordinator **and** that movement's `ending_time` |
| completion | `Proceed` at the coordinator's cycle time | panics if it would allocate |
| none (frame scope) | unchanged, gated on a live frame | unchanged; allocates on the task's clock |

`task_context::animation_time()` reads the callback scope first and `ANIMATION`
otherwise, so `AmbientTimeScale` and any other time-based ambient access also
work inside a callback. Panic diagnostics name the callback kind; with `panic =
"abort"` a panic is a hard stop rather than a catchable error, so the message is
the entire diagnostic. `animate_if_changed` on an unchanged target returns
before allocating, so the no-op rule and the panic rule cannot collide.

#### Client API surface

Application authors, via `massive_applications::prelude`:

```rust
pub fn movement<T, F>(value: T, apply_animations: F) -> TaskMovementBuilder<T, F>

impl<T, F> TaskMovementBuilder<T, F> {
    pub fn completion_event<E, G>(self, completion_event: G) -> Self;
    pub fn mount(self) -> TaskMovement<T>;
}

impl<T> TaskMovement<T> {
    pub fn modify(&self, modifier: impl FnOnce(&mut T) + Send + Sync + 'static);
    pub fn snap(&self);
}
// Drop queues the unregister action.
```

`modify` receives only the movement value: no progress, no allocator. Sealing is
the point — no accessor to `Movement<T>`, no `Deref`, and no constructor other
than `mount()`. Shell and lifecycle integration keeps
`task_context::apply_movement_animations` and
`task_context::end_frame_cycle_detached`; the broad
`with_animation_and_movement` is removed. Mounting and queueing stay ungated,
since they only touch the inbox. Six files carry the presenter-side edits:
`desktop/src/desktop_presenter.rs`,
`desktop/src/desktop_system/focus_depth_indicator.rs`,
`desktop/src/instance_presenter.rs`,
`desktop/src/projects/launcher_presenter.rs`,
`desktop/src/projects/project_presenter.rs`, and
`examples/logs/examples/logs.rs`. Low-level integration that builds a
`TaskContext` directly keeps working unchanged:
`desktop/src/instance_manager.rs`, `shell/src/shell.rs`, and the tests in
`applications/src/task_context.rs`, `applications/src/task_context/shaper.rs`,
and `examples/markdown/src/lib.rs`.

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

## Open questions

**Q1 — Session scope.** Phase 1 + validation only, no shell or instance lifecycle
changes?
➡️ Yes: the drain changes when an instance task lives, which is a separate
question, and it depends on the callback API being settled first.

**Q2 — Type identity of the task handle.** A `TaskMovement<T>` newtype in
`applications` (recommended: the animation crate keeps a fully explicit API, and
the `Task*` prefix matches `TaskMovementBuilder` / `TaskContext`), versus keeping
`Movement<T>` as the field type with the one-arg form as a free function.
Keeping the same type is mechanically possible, but it leaves presenters able to
reach the explicit `modify(|value, context| ..)` form through the
`massive_animation` re-export, so the "no bypass" validation item becomes
unmeetable by construction. Two same-named `modify` methods cannot coexist on
one type, and an inherent `modify` wins over a same-named trait method regardless
of arity.

**Q3 — Is the `CallbackScope` seam accepted?** It adds a parameter to both
sweeps and new public vocabulary to the animation crate. The alternative
considered and rejected was wrapping each callback in the task layer, which
contradicts "installed once per sweep" and would need the allocator to escape the
runtime.

**Q4 — Allocation rules.** Modifier-only allocation; coordinator and
`ending_time` both updated; panic for apply and completion;
`animate_if_changed` unchanged-target no-op; frame-scoped behavior outside
callbacks unchanged.
➡️ As stated. The completion case needs the panic too, because
`apply_animations` clears `ending_time` before invoking the completion callback,
so an animation allocated there would never advance and never emit a second
event.

**Q5 — Detached-sweep cycle.** `flush_and_end_cycle` runs `run_actions` with no
open cycle, and a queued modifier's allocation then panics inside
`InstanceContext::drop` under `panic = "abort"`. Options: (a) leave it, (b) have
the sweep own a cycle — `flush_and_end_cycle` begins one if none is open,
(c) skip queued modifiers in a detached sweep.
➡️ (b), in phase 1. `begin_cycle` is `get_or_insert_with`, so it is idempotent
on the frame path and changes no pacing there, while on the detached path it
turns "a dying instance started an animation" into a phase-2 question instead of
a process abort.

**Q6 — Apply callback signature.** Keep `FnMut(&mut T, AnimationProgress)`
(recommended: progress is a mandatory input to every apply body, so it is not the
ceremony the allocator was, and it keeps the exact-progress contract in the type
rather than in ambient state — the very property allocation is being fixed for),
or drop to `FnMut(&mut T)` and read progress ambiently. The scope announces
`Apply(progress)` either way, because helpers called from an apply callback need
it.

**Q7 — Tests.** (i) `massive-animation` unit tests with a fake `CallbackScope`
pinning the seam: one announce per callback, correct kind, exact
`AnimationProgress` for apply/snap, cycle instant for modifier/completion, once
per sweep rather than once per queued modifier. (ii) `applications`
`#[tokio::test]` tests pinning routing: apply sees exact progress, modifier and
completion see cycle time, frame-scoped behavior outside callbacks is unchanged,
modifier allocation updates coordinator and `ending_time`, apply/completion
allocation panics with the diagnostic, and a queued modifier in a detached sweep
no longer panics. (iii) "consumers cannot reach the runtime" asserted as API
absence — there is no trybuild or compile-fail infrastructure in the tree.

**Q8 — Documentation and naming.** Record the new seam as a new ADR (0009): a
public framework-neutral hook plus a task-side callback scope is a distinct
decision from ADR 0008's task-locals. Add the new terms to `GLOSSARY.md`. ADR
0008's Context section still names `with_coordinator_and_movement`, which no
longer exists; fix that in the same pass. Names to confirm: `TaskMovement<T>`,
`CallbackScope`, `MovementCallback<'_>`, `apply_movement_animations`. Keep a
status line in this plan until phase 2 exists.

## Sequencing

1. Implement callback-local task API and runtime batch support: the sweep hook in
   `massive-animation`, the task-side scope installed once per sweep,
   task-specific movement handles, callback scope for all callback kinds,
   progress routing, modifier-only allocation, narrower task operations, and the
   detached-sweep cycle fix (Q5). Keep the low-level runtime reusable and
   framework-neutral.
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
- Confirm a queued modifier in the detached teardown sweep no longer panics
  inside `InstanceContext::drop`.
- For the deferred drain, verify close and shutdown keep an instance alive to
  `Settled`, submit final changes, and terminate; separately verify expiry of
  the global `Shutdown deadline` ends waiting even if an instance is unsettled.

## Not part of this plan

The failing test in `shapes/src/shaping_engines/cosmic_scratch.rs`
(`a_failed_resolution_is_not_retried`) belongs to the shaping-session workstream
and is unrelated to this plan.
