# Task-local UI contexts

## Status: proposed; implementation phased

## Problem

The UI layer threads several contexts through presenters and call sites even though their ownership is already per task: the scene, animation coordinator, movement runtime, and shaping scratch contexts. This creates mechanical parameters across roughly 15 presenter constructors and 30 call sites in the examples, `mt`, and the desktop without adding useful ownership information.

The renderer's font registry, the pty `Arc<Mutex<Terminal>>`, and the movement action inbox remain shared because their ownership is cross-thread by design. Domain state (`ViewState`, presenter state, and window or renderer handles) also remains explicit.

## Decision

Install task-owned UI contexts once per Tokio task and expose narrow accessors for them. Task-local storage is an access mechanism only: it does not change ownership, submission ordering, or the explicit state passed through the UI.

### Task-local storage

`massive-applications` owns the task-local context module, and `massive-shell` re-exports it for shell users. It provides:

- a bare `Scene` task-local, since scene methods take `&self` and mutation flows through its internally shared change collector;
- `RefCell<Option<AnimationCoordinator>>` and `RefCell<Option<MovementRuntime>>` task-locals for contexts currently passed as `&mut` values; and
- an `Arc<Mutex<ShaperContext>>` task-local for per-task font and layout scratch.

Accessors return owned values or guards rather than references. Tokio task-local borrows cannot escape `with`; `Scene` therefore uses a cheap owned clone, while mutable contexts use owned guards. Re-entrant mutable access must use a non-blocking acquisition and fail loudly instead of deadlocking.

The context is installed at existing task boundaries: `InstanceManager::spawn` for instance tasks and `shell::run` for the application task. Spawned sibling tasks do not inherit a parent's context and must install their own contexts. There is no default scene: an accessor without an explicitly installed collector-backed scene panics and identifies the missing installation point. This preserves the instance requirement that scene changes and instance changes share one ordered submission stream.

### Frame as per-cycle state

`Frame` becomes per-cycle state rather than a container for borrowed scene, animation, and movement contexts. Constructing a frame opens the animation cycle and obtains the task-local contexts. A live frame prevents another frame from being constructed in the same task; the existing "frame dropped without being submitted" guard remains. Per-cycle scratch state stays on `Frame`.

The explicit scene entry points remain available for multi-scene tasks and tests: `enter(&scene)`, `Frame::scene()`, and `new_scene_with_change_collector`. `Movement::modify` also retains its inbox mutex because movement handles can outlive a task context and the inbox is a cross-boundary mailbox.

### Shaping split

`FontManager` retains the shared font registry, source cache, face-data map, generic family mapping, and symbol-fallback rebuild. The renderer continues to receive a `FontManager` handle. Per-task `ShaperContext` values hold `FontContext` and `LayoutContext<GlyphBrush>` over the shared collection, so shaping scratch is not serialized through the registry lock. The generic-family mapping remains part of shared collection setup because omitting it can produce silently empty layouts.

The existing `.shape(&mut shaper)` builder API and the small number of raw `contexts()` call sites remain usable through the task-local guard. Completion-event downcasting and the app-configuration constructors `FontManager::bare`, `with_font`, and `system` are unchanged.

## Context and invariants

- Context access is synchronous and confined to one poll; guards cannot cross an `await`.
- Task boundaries are firewalls. Every `tokio::spawn`, `JoinSet::spawn`, and blocking-thread site that uses these contexts must install its own scope.
- Nested task contexts are supported; an inner context shadows and restores the outer value.
- `RefCell` re-entrancy and `try_lock_arc` failures are deliberate loud errors for nested frames and shaper acquisition.
- The movement inbox remains explicit shared state, not ambient task-local state.
- `massive-animation` does not gain a Tokio dependency merely to define the task locals; its low-level APIs retain explicit context parameters until call sites are migrated.
- The task-local implementation must compile for native and wasm32 targets. Tokio's `rt` feature is required by `task_local!` and is already enabled for wasm32 in `massive-shell`.

## Considered options

- **Thread every context explicitly.** Rejected for the UI layer: ownership is already per task, and the repeated parameters are mechanical plumbing. Explicit scene entry points and domain state remain where they carry meaningful information.
- **Use a shell-level default scene.** Rejected: it could route changes into the wrong collector and violate submission ordering. Missing installation is an error instead.
- **Use a mutex for every mutable task-local.** Rejected: `RefCell` makes nested frame access fail immediately and prevents a borrowed reference from crossing an `await`; the shaper uses an owned mutex guard because its guard must escape the task-local closure.
- **Make the whole font manager task-local.** Rejected: the registry and font data are intentionally shared with the renderer; only shaping scratch is task-owned.
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

Native and wasm32 checks are required in phase 1. Each migration phase should run the `massive` workspace and `mt` checks, the examples suite, and a smoke test of scroll animation and hyperlink hover.

## Evidence

The design was compile-verified against Tokio 1.48, `parking_lot` 0.12.5 with `arc_lock`, parley 0.11.1, and fontique 0.11.1. The checks established that task-local values are isolated across spawned tasks, nested scopes shadow and restore values, references cannot escape `LocalKey::with`, owned `ArcMutexGuard` values can support mutable access, and the relevant shaping contexts are `Send + Sync`. They also confirmed that a shared font collection without its generic-family mapping can shape to an empty layout without returning an error.


## Final Tasks (added by the author), to be removed after realized.

- Remove Scene cloning.
