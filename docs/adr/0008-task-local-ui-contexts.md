# Task-local UI contexts

## Status: implemented

## Problem

The UI layer threads several contexts through presenters and call sites even though their ownership is already per task: the scene, animation coordinator, movement runtime, and shaping scratch contexts. This creates mechanical parameters across roughly 15 presenter constructors and 30 call sites in the examples, `mt`, and the desktop without adding useful ownership information.

The renderer's font registry, the pty `Arc<Mutex<Terminal>>`, and the movement action inbox remain shared because their ownership is cross-thread by design. Domain state (`ViewState`, presenter state, and window or renderer handles) also remains explicit.

## Decision

Install task-owned UI contexts once per Tokio task and expose narrow accessors for them. Task-local storage is an access mechanism only: it does not change ownership, submission ordering, or the explicit state passed through the UI.

### Task-local storage

`massive-applications` owns the task-local context module, and `massive-shell` re-exports it for shell users. It provides:

- a bare `Scene` task-local, since scene methods take `&self` and mutation flows through its internally shared change collector;
- `RefCell<AnimationCoordinator>` and `RefCell<MovementRuntime>` task-locals for contexts currently passed as `&mut` values; and
- one `TaskContext` that always carries a shaping context. This supersedes both the shape-free application context and the separate `InstanceTaskContext` of the original decision; see the amendment below.

Accessors return owned values or guards rather than references. Tokio task-local borrows cannot escape `with`; `Scene` therefore uses a cheap owned clone, while mutable contexts use owned guards. Re-entrant mutable access must use a non-blocking acquisition and fail loudly instead of deadlocking.

The contexts are installed at existing task boundaries: `InstanceManager::spawn` for instance tasks and `shell::run` for the application task. The instance factory is invoked after its context is installed, so initialization and the returned future observe the same task-local values. Spawned sibling tasks do not inherit a parent's context and must install their own contexts. There is no default scene or shaping context: an accessor without an explicitly installed value panics and identifies the missing installation point. This preserves the instance requirement that scene changes and instance changes share one ordered submission queue.

### Frame as per-cycle state

`Frame` becomes per-cycle state rather than a container for borrowed scene, animation, and movement contexts. Constructing a frame opens the animation cycle and obtains the task-local contexts. A live frame prevents another frame from being constructed in the same task; the existing "frame dropped without being submitted" guard remains. Per-cycle scratch state stays on `Frame`.

The explicit scene entry points remain available for multi-scene tasks and tests: `enter(&scene)`, `Frame::scene()`, and `new_scene_with_change_collector`. `Movement::modify` also retains its inbox mutex because movement handles can outlive a task context and the inbox is a cross-boundary mailbox.

### Shaping split

`FontManager` retains the shared font registry, source cache, face-data map, generic family mapping, and symbol-fallback rebuild. The renderer continues to receive a `FontManager` handle. Per-task `ShaperContext` values hold `FontContext` and `LayoutContext<GlyphBrush>` over the shared collection, so shaping scratch is not serialized through the registry lock. The generic-family mapping remains part of shared collection setup because omitting it can produce silently empty layouts.

The sizing builder resolves the task's shaper itself: `label.size(FONT_SIZE).shape()` shapes through `AmbientShape` (see the ambient-shaping amendment below), while `SizedTextShaper::shape_with(&mut Shaper)` stays the entry point for callers that already hold a shaper. Completion-event downcasting and the app-configuration constructors `FontManager::bare`, `with_font`, and `system` are unchanged.

## Context and invariants

- Context access is synchronous and confined to one poll; guards cannot cross an `await`.
- Task boundaries are firewalls. Every `tokio::spawn`, `JoinSet::spawn`, and blocking-thread site that uses these contexts must install its own scope.
- Nested task contexts are supported; an inner context shadows and restores the outer value.
- `RefCell` re-entrancy, missing task-local contexts, and shaper acquisition failures are deliberate loud errors.
- The movement inbox remains explicit shared state, not ambient task-local state.
- `massive-animation` does not gain a Tokio dependency merely to define the task locals; its low-level APIs retain explicit context parameters until call sites are migrated.

## Considered options

- **Thread every context explicitly.** Rejected for the UI layer: ownership is already per task, and the repeated parameters are mechanical plumbing. Explicit scene entry points and domain state remain where they carry meaningful information.
- **Use a shell-level default scene.** Rejected: it could route changes into the wrong collector and violate submission ordering. Missing installation is an error instead.
- **Use a mutex for every mutable task-local.** Rejected: `RefCell` makes nested frame access fail immediately and prevents a borrowed reference from crossing an `await`; the shaper uses an owned mutex guard because its guard must escape the task-local closure.
- **Make the whole font manager task-local.** Rejected: the registry and font data are intentionally shared with the renderer; only shaping scratch is task-owned.
- **Use one context type for application and instance tasks.** Rejected for now: application tasks intentionally remain shape-free, while instance tasks require a fresh shaping context.
- **Restructure the runtime around custom task types.** Rejected: existing Tokio task boundaries already match the ownership model.

## Consequences

- Presenter constructors and call sites no longer carry task-owned scene, animation, movement, or shaping parameters after migration.
- Misconfigured task boundaries fail at the access point rather than silently using a separate collector or blocking on re-entrant locks.
- The API trades some compile-time ownership checking for runtime-checked context installation. The context rules and focused tests are therefore part of the design, not incidental implementation detail.
- Per-task shaping scratch can proceed concurrently while registry operations remain synchronized and renderer reads remain shared.
- Tests and multi-scene code can continue using explicit scene installation.

## Implementation notes

The migration is additive at first and proceeds in these phases:

1. Add `massive-applications::task_context` and re-export it from `massive-shell`; port tests for scene access, mutable guards, sibling isolation, `Send` bounds, nested-frame rejection, and re-entrancy.
2. Make `Frame` obtain the clock and movement runtime from the scope while preserving its submission and drop invariants.
3. Add ambient scene entry alongside the explicit scene APIs.
4. Add the runtime-free `task_context::movement` builder and migrate movement consumers and desktop presenters.
5. Split shaping scratch from the shared `FontManager` registry and migrate the four raw `contexts()` call sites.
6. Migrate examples, `mt`, and desktop, then remove obsolete threaded parameters.

Each migration phase runs the `massive` workspace and `mt` checks, the examples suite, and a smoke test of scroll animation and hyperlink hover.

## Evidence

The design was compile-verified against Tokio 1.48, `parking_lot` 0.12.5 with `arc_lock`, parley 0.11.1, and fontique 0.11.1. The checks established that task-local values are isolated across spawned tasks, nested scopes shadow and restore values, references cannot escape `LocalKey::with`, owned `ArcMutexGuard` values can support mutable access, and the relevant shaping contexts are `Send + Sync`. They also confirmed that a shared font collection without its generic-family mapping can shape to an empty layout without returning an error.


## Finalization Tasks

- [x] Simplify `FontManager`: shared authority/publication state is separated from per-owner
	`ShapingContext` scratch.
- [x] Keep scene cloning explicit with `clone_scene()`. Scene values still need a cheap handle
	clone when a task-local collector is installed; implicit `Clone` is not required.
- [x] Keep the application and instance task contexts separate; instance scenes and shaping
	scratch are owned by `InstanceTaskContext`. (Superseded by the amendment below: one task
	context type; an instance still owns its scene and shaping scratch.)
- [x] Install one `ShapingContext` per instance task. Font loading remains available through the
	instance's shared `FontManager`, while shaping scratch is task-local.
- [x] Remove font shaping state from `TerminalViewParams`; terminal line shaping uses the instance
	task's shaper context.
- [x] Expose the shaping engine kind from the manager, shaper, and published registry.
- [x] Keep `PublishedRegistry.kind`: the renderer compares it with each `GlyphRun`'s shaping kind
	at the render boundary to catch cross-engine misuse.

## Amendment: one task context, shaping always present (2026-09-21)

The split this document decided is superseded: application and instance tasks share one task context
type, which always carries a shaping context. The separate instance context and its installer are
gone.

The reason is that the split's premise did not survive. Application tasks were to remain shape-free;
the desktop's application task shapes (its focus-depth badges), so the distinction described no task
that exists. What it still cost was a second type whose fields were a superset of the first, a second
installer, and an optional shaping context in the task-local — which made `with_shaper` fail for a
state that can no longer occur. One type leaves one installer and one invariant: every UI task has
exactly one shaping owner.

The ownership rule the split protected survives: an instance still owns its own scene and its own
shaping scratch. The rejected option below — "use one context type for application and instance
tasks" — is therefore revised, not the reasoning that rejected it, which turned on application tasks
being shape-free.

The rest of this document stands unchanged: task-local storage is an access mechanism, the contexts
are installed at existing task boundaries, sibling tasks do not inherit a context, missing
installation is a loud error, and the font manager stays shared rather than task-local. The manager a
task shapes with is fixed when its context is installed (see the font-policy amendment in
[ADR 0005](0005-dual-shaping-engines-cosmic-text-and-parley.md)).

## Amendment: one change queue per task, the scene type removed (2026-09-23)

The explicit scene entry points this document promised — `enter(&scene)`,
`Frame::scene()`, `new_scene_with_change_collector` — are gone, and so is the
`massive_scene::Scene` type they belonged to. A scene, it turned out, was a
one-field wrapper around an erased change receiver: every one of its operations
was a forwarder. What tasks actually own is a change queue, and each queue has
exactly one change type: `SceneChange` for the application task's render queue,
`InstanceChange` for an instance's submission queue (which interleaves scene
changes via `From<SceneChange>`).

### Decision

- `massive_scene::AnyCollector` is the task's change queue: type-erased so one
  task-local can hold any collector kind, with the change type fixed at install
  time by `for_type::<C>()`. Typed accessors downcast and panic loudly on a kind
  mismatch, naming the requested type. The erased sink is all that `Handle<T>`
  ever sees — that erasure at the handle is what makes one queue per task
  sufficient, because a handle's writes are retyped into the task's queue (the
  `InstanceChange::End` ordering guarantee of the desktop relies on this).
- Task accessors are write-only: `enter` and `collect`. The only
  drain is `task_context::take_changes::<C>()`, invoked by the frame at
  submission time. This preserves the rule that task-local storage is an access
  mechanism and does not move the submission boundary: the drain still happens
  where pacing and final-submission-on-drop are owned.
- `Frame<C>` carries the change type as its only state. `render_submission()`
  exists only where `C = SceneChange`; an instance frame drains into an
  `InstanceChange` set for `InstanceContext::submit`. Draining a queue into the
  wrong submission kind is a compile error, replacing the previous runtime panic
  waiting in `Scene::take_changes` on instance collectors.
- The send-only object-safe `ChangeSink` trait, with a blanket implementation for
  every `ChangeCollector<C>` whose change type embeds scene changes, replaces
  `HandleChangeReceiver` (whose `take_changes` defaulted to a panic). The
  instance change collector is now the plain generic collector plus
  `From<SceneChange> for InstanceChange`; the orphan-rule newtype is gone.
- Method-style entry is one trait: `Enter::enter()` for scene values. It is
  implemented for the object types and for `UnenteredLocation`, which enters
  together with its transform in one batch; the trait dispatches by type because
  a blanket `impl<T: Object>` would conflict with the `UnenteredLocation` impl (a
  downstream crate may add `Object` for it). Code that must name the collector
  instead of using the task's — tests, multi-queue tasks — calls
  `scene::enter(collector, value)` or `UnenteredLocation::enter_in(collector)`.

### Consequences

- The desktop's separate per-desktop change collector is gone: the application
  task's installed queue is the desktop's render queue, so the
  previously-installed-but-never-drained shell scene can no longer exist.
- An undrained queue at task end is a `ChangeSet` drop log, not silent loss;
  since the installed queue is now live for every application, "a task's queue
  is drained or the task logs" is an invariant, not an implementation detail.
- Criterion benchmarks (`massive/scene/benches/push_cost.rs`) pin the erasure
  cost: roughly 2 ns per push over the plain typed collector, about 10%.

## Amendment: ambient shaping, one ambient module (2026-09-23)

Text shaping joins scene entry on the ambient path: `label.size(FONT_SIZE).shape()`
resolves the task's shaper at the call site, so no presenter or call site carries a
`&ShapingContext` any more.

### Decision

- The ambient step is `AmbientShape::shape(self) -> Option<GlyphRun>`, implemented
  for `SizedTextShaper` — the type the sizing chain actually produces. It opens the
  shaper from the task-local and delegates.
- `SizedTextShaper::shape(self, &mut Shaper)` becomes `shape_with`. Two names, two
  cases: `shape()` is ambient, `shape_with(shaper)` takes the shaper the caller
  already holds (tests, benchmarks, shaping outside a task). The rename is forced by
  name resolution, not taste: method lookup matches the receiver type and ignores
  arity, so an inherent `shape` wins the probe against a same-named trait method even
  when only the trait's arity fits. The compiler reports a missing argument
  (`E0061`) for the inherent method, and nothing points at the trait.
- The trait lives in `massive-applications`, which owns the task-local, not in
  `massive-shapes`. `massive-shapes` sits below the context's installation point and
  cannot name it without inverting the dependency. This gives `massive-applications`
  a direct `massive-shapes` dependency, replacing its accidental reach through
  `massive-renderer`'s re-exports.
- Ambient accessors share one module, `massive-applications::ambient`: `Enter` and
  `AmbientShape` are the same idea — read the task's installed context instead of
  threading it — and the `prelude` re-exports them together. The earlier
  `enter_ambient` and `ambient_shape` modules are merged into it.

## Amendment: one identity world per task (2026-09-25)

Every UI task has exactly one font manager — the one the shell built from the `FontPolicy` — and
the renderer reads that manager's registry. The task reaches it through `fonts()`, a free accessor
beside `with_shaper` in the task-context module and the prelude, because the manager is a property
of the task and not of a context: the contexts handed to applications carry no task-owned state.
Fonts an application needs are loaded into that manager, which ADR 0006 permits at any time; the
shaper is pulled from the same task-local at the shaping call, so its exclusive scratch is held for
the shape and nothing else.

### Decision

- The task's manager is reached through `fonts()`, for application and instance tasks alike. No
  context exposes a `fonts()` method, so the access surface is uniform and the contexts stay the
  shell's handle.
- `WindowRendererBuilder::with_text()` is the single way to enable text rendering, and it resolves
  the registry from the task-local. A renderer whose text layer observes a different manager's
  registry than the one that shaped its runs cannot be correct — a `FaceId` is meaningful only
  inside the issuing manager (ADR 0005) — so there is no variant that takes a registry.
- No production code constructs a `FontManager`; tests and benchmarks do, and they have no task
  context. A manager built outside the shell is a second identity world no renderer can consume.
- The requirement is *one identity world per task*, not *every shaping call goes through the
  task-local*: every `GlyphRun` a task renders was shaped through that task's manager, or converted
  from a foreign layout engine and mapped to that manager's face ids, which the renderer checks by
  comparing each run's shaping engine with its layer's (ADR 0005). A foreign layout engine may
  therefore keep shaping.
- Frames are opened by one prelude free function, `begin_frame()`, and `Frame` carries no change
  kind as a type parameter: the kind is fixed where the frame is consumed — render submissions
  drain the application task's `SceneChange` queue, and an instance's `submit` fixes
  `InstanceChange`. A kind on the opener cannot do that: it forces an annotation exactly where
  nothing else constrains the kind, and one opener per task kind duplicates the entry point.
- Draining a queue with the wrong kind is therefore a runtime failure rather than a type error, and
  it is a loud one: the typed task-local accessor panics naming the requested type (the
  one-change-queue-per-task amendment above).

## Amendment: the frame witnesses animation access (2026-09-25)

Animation state becomes reachable only while a frame is live. The task local holds one
`ANIMATION: RefCell<AnimationState>` whose fields are the `AnimationCoordinator`, the
`MovementRuntime`, and the frame *witness* (`Option<FrameWitness>`). `begin_frame()` installs the
witness and releases it unconditionally via `Frame`'s `Drop`; the owned `Frame` itself never
enters the task local. Its remaining fields are diagnostics only: `submitted` is what lets Drop
distinguish a commit from a leak, `created_at` locates the leak.

### Why the witness records the creation site

Occupancy alone (`Cell<bool>`) would be enough for the gate. `FrameWitness.created_at` exists for
exactly one panic only the witness can produce: a second `begin_frame()` while a frame is still
live. The live frame is a plain local further up the stack the task local deliberately does not
own, so only the recorded site can name the blocking frame unconditionally; without it the panic
points at the second call site and the culprit must be recovered from a backtrace.

### Why the animation world is one cell

The coordinator, the movement runtime, and the witness share a cell because they are accessed
together: every `run_actions`/`apply_animations` step drives the coordinator by the movement
runtime's `ending_time`, and the witness gates access to both. Separate cells made each access pay
a second task-local lookup and a second `try_borrow_mut` with its own re-entrancy panic, and they
forced `begin_frame` to acquire the witness and then separately begin the cycle — a two-step
opener whose ordering ("acquire before touching animation state") had to be documented as
load-bearing. One cell makes the gate a field check inside the single borrow that already yields
the coordinator, and collapses the opener into `begin_frame_cycle`, which stores the witness and
begins the cycle under one borrow.

### Decision

- One task-local owner (`ANIMATION: RefCell<AnimationState>`) holds the coordinator, the movement
  runtime, and the witness. All mutation and timestamp reads go through one borrow point,
  `with_animation_state`, which panics on re-entry. `with_frame_animation` and `end_frame_cycle`
  assert a live frame and hand out the coordinator field, respectively end the cycle; there is no
  ungated coordinator tier — the coordinator's query methods are reached only through the gated
  accessors or test code.
- `begin_frame_cycle(created_at)` is the only opener: it installs the witness and begins the
  cycle, and returns where the blocking frame was begun when one is already live. `begin_frame()`
  turns that into the panic naming both frames' creation sites. This implements the "a live frame
  prevents another frame" rule from the original decision, which was previously unimplemented:
  `begin_cycle` silently reused an open cycle, dating new animations against a stale cycle start
  time.
- `Frame::drop` is release-only and unconditional: it releases the witness, and for a frame
  dropped without submitting it closes the open cycle (flushing movement actions first) and logs
  the leak. Frames are legitimately dropped un-submitted on the normal quit paths (the desktop's
  and instance loops return on `CloseRequested`/`Shutdown` with a frame open, and shutdown opens
  further frames afterwards), so a missing submit must never poison the witness or trap the next
  begin — that is why the witness is released rather than left for a detection-at-next-begin.
- One `&mut Frame`-passing boundary survives deliberately: the commit sites (`Frame::submission`,
  `Frame::render_submission`) take the frame by value, so a commit is still impossible without
  owning the frame. Everything else reads the ambient clock: `task_context::animation_time()` and
  `task_context::allocate_animation_time(duration)` replace every `&mut Frame`/
  `frame.animation_time()` parameter (in `transact`, `handle_instance_ended`,
  `TerminalPresenter::update`, `process_view_event`, selection progress). There is no allocator
  object on the ambient path: `AmbientAnimation::animate` reads the allocated start time from the
  task's frame and calls `Animated::animate_at(instant, ..)`, the same explicit-timestamp step
  `animate_with` reaches after delegating allocation to its `context`.
- Two sanctioned accesses run without a live frame, one named function each: (1) the shell's
  apply-animations step while awaiting events (`ApplicationContext::wait_for_events`),
  `task_context::with_coordinator_and_movement`, which upgrades the cycle, applies movement
  runtime animations and returns their completion events — the cycle deliberately stays open,
  because the cycle spans the shell's step and the application's next frame, and `end_cycle`'s
  `ApplyAnimations` comparison needs that span to decide pacing; (2) instance teardown
  (`InstanceContext::drop`), `task_context::end_frame_cycle_detached`, which closes the cycle for
  the final pacing decision and flushes queued movement actions — a real cycle end, not a silent
  `end_cycle`-with-no-cycle no-op. Neither installs a frame witness. An earlier design did, to
  "witness" the access, and took a closure so callers could supply the work; both were removed:
  the witness cannot be consulted from inside these spans, because the call holds the single
  `with_animation_state` borrow for its whole duration, so any gated access from within panics on
  re-entry (`animation state was re-entered`) before the gate is ever read, and opening a frame
  from within is impossible by construction (the application task owns the frame loop and is
  blocked in the span; `begin_frame_cycle` could not borrow the state anyway). A witness would
  also have to be installed *before* the borrow, re-introducing the split borrow the single-borrow
  design removed. With no closure to fill, the step reads its two fields directly and the shell
  calls one operation instead of restating the coordinator/movement pairing.
- Frame ends share one body, `AnimationState::flush_and_end_cycle`: queued movement actions are
  flushed before the cycle closes, because completion events arrive during apply-animations cycles
  and may queue successor actions that must not wait for an unrelated event. It is reached through
  `end_frame_cycle` (gated, for `Frame::submission` and `Frame::drop` — the cycle that closes is
  the one that decides the next frame's pacing) or `end_frame_cycle_detached` (teardown).
- Mounting and queueing movements stay ungated: `movement(..).mount()`, `Movement::modify`, and
  `Movement::snap` touch only the movement runtime's action inbox, never the clock — mounting
  happens at presenter construction, before any frame exists. `mount` reaches its field directly
  inside `with_animation_state`, which is also the only borrow point; `with_movement` and
  `with_animation_and_movement` existed for exactly two callers each and asserted nothing the
  borrow point did not, so they are inlined. The inbox defers actions to the next frame's
  `run_actions` by construction (the cross-boundary mailbox the ADR already grants). `Frame` is
  `!Send` so a witness release can never fire on behalf of another task.
- Movement closures keep the movement's own allocator. `MovementRuntime::run_actions` wraps each
  queued `modify` in a `MovementAnimationAllocator` that records the movement's `ending_time`,
  and `apply_animations` advances only movements that have one. A closure that allocated its
  animations on the ambient clock would leave `ending_time` unset, so those animations would
  never advance and never emit a completion event; presenter and example call sites inside
  `modify` therefore keep the explicit `animate_with`/`proceed_with` forms. The ambient API is the
  default everywhere else.

### Consequences

- `begin_frame()` is the only opener and the witness gate is the only path to animation state; a
  mis-ordered call chain fails with a panic naming the missing frame instead of silently reading a
  stale or default cycle time.
- Text rendering requires an installed task context, which every application and instance task has.
- `FontManager::bare`/`system`/`with_font` are the construction path for non-task code.
- `begin_frame`'s signature and the submission call sites are unchanged.
