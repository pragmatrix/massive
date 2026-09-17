# FontManager simplification plan

## Goal

Reduce ownership and synchronization concepts in `FontManager` without changing the
per-instance shaping model from ADR 0006:

- one shared face authority issues `FaceId` values;
- one published registry is readable without the authority mutex; and
- each logical owner has exclusive shaping scratch.

The plan separates those roles more clearly and removes synchronization primitives
that do not represent real sharing.

## Current braid

`FontManager` currently combines three roles:

1. `inner`: the shared, mutex-protected face authority;
2. `published`: the shared atomic registry publication channel; and
3. `scratch`: one handle's exclusive shaping state.

The first two are intentionally shared. The third is not: `FontManager` is not
`Clone`, and `detached()` creates a fresh scratch value for each owner. The type
therefore currently uses `Arc` around the scratch mutex even though that mutex is
not shared.

Task-local shaping also has two exclusivity mechanisms: `task_context` wraps a
`FontManager` in `RefCell`, while `FontManager::shaper()` protects scratch with a
non-blocking mutex. The public manager API already uses `&self`, so the `RefCell`
is mostly mechanical borrowing protection.

## Proposed direction

### 1. Make scratch singly owned

Change the handle field from:

```rust
Arc<Mutex<Box<dyn EngineScratch>>>
```

to:

```rust
Mutex<Box<dyn EngineScratch>>
```

Keep the mutex because a handle still needs runtime detection of two simultaneous
shapers. Remove only the outer `Arc`; `detached()` continues to construct a new
mutex and new scratch for each logical owner.

### 2. Remove the task-local `RefCell` around `FontManager`

Store `FontManager` directly in the shaping task-local and expose it to callers as
`&FontManager`. Let the scratch mutex remain the single re-entrancy mechanism.

This preserves the existing loud failure for nested shaping while eliminating a
second, independent borrow protocol. Update callers that currently accept
`&mut FontManager` only because of the `RefCell` wrapper.

### 3. Bundle publication identity

Introduce a shared publication object containing the engine kind and atomic registry
cell, conceptually:

```rust
struct PublishedRegistry {
    kind: ShapingEngineKind,
    current: ArcSwap<FontRegistry>,
}
```

Use `Arc<PublishedRegistry>` for the manager and `FontRegistrySource`. This keeps
the necessary outer `Arc` around `ArcSwap`: detached handles and renderers must
observe the same swap cell. It also prevents `kind` and `published` from becoming
mismatched parallel fields.

`FontRegistrySource` remains a cheap, render-only capability. It must not gain the
face authority or shaping scratch.

### 4. Name the face authority directly

Rename `FontManagerInner` and the `inner` field to reflect their domain role, for
example `FontAuthority` and `authority`. The authority owns the canonical engine
used for loading and fallback-face resolution; it does not own per-frame shaping
scratch.

This is a naming and boundary clarification, not a new synchronization layer.

### 5. Narrow the engine contract where possible

Audit `ShapingEngine` methods that are no longer used by production code after the
per-handle scratch split, especially canonical-engine `shape()` and `font_data()`.
Remove them only when tests and all engine implementations no longer require them.
Keep registration, publication, scratch construction, and fallback resolution as
the explicit engine capabilities.

### 6. Make session registry reads coherent

`Shaper::font_data()` currently checks its session snapshot and then falls back to
the manager's latest publication, while `metrics()` reads only the session snapshot.
Prefer one coherent session snapshot for both methods. Preserve the existing refresh
after `shape()`, and add a regression test that every face in a returned run is
available from that snapshot.

## Implementation phases

1. **Scratch ownership:** remove `Arc` from the scratch field and update constructors.
2. **Task-local access:** remove the `RefCell` wrapper and migrate the small set of
   `with_shaper` callers from `&mut FontManager` to `&FontManager`.
3. **Publication value:** introduce `PublishedRegistry`, migrate manager and source
   reads/writes, and preserve the outer `Arc` around the shared swap cell.
4. **Authority naming:** rename `FontManagerInner` and its field without changing
   behavior.
5. **Contract cleanup:** remove genuinely unused `ShapingEngine` methods and make
   session registry lookup consistent.
6. **Validation and documentation:** update ADR 0006 only if its ownership wording
   changes, then run focused tests followed by the `massive` workspace and `mt`
   checks.

Each phase should remain a separately compilable change. Do not combine the
publication refactor with behavioral changes to registry freshness or shaper
lifetime.

## Invariants to preserve

- `FaceId` values come from one canonical authority.
- Registration and fallback resolution publish a complete immutable registry.
- Renderer reads remain lock-free and observe the shared publication channel.
- Different detached handles shape concurrently.
- Two shapers on one handle fail immediately rather than block indefinitely.
- A shaper refreshes its session snapshot after shaping before callers inspect the
  returned run.
- `FontRegistrySource` remains render-only.

## Non-goals

- Do not make `FontManager` clonable; `detached()` intentionally makes ownership
  boundaries visible.
- Do not replace the atomic snapshot publication with a mutex or a broadcast list.
- Do not merge the face authority and per-owner scratch back into one engine object.
- Do not change the accepted ADR 0006 runtime exclusivity policy in this cleanup.

## Validation

The focused checks should cover:

- detached managers have independent scratch and can shape concurrently;
- re-entrant shaping on one handle still panics at `shaper()`;
- loads and fallback resolution remain visible through all registry sources;
- renderer registry reads remain lock-free from the caller's perspective; and
- session `font_data` and `metrics` resolve every face carried by a shaped run.

After those checks, run the existing `massive` workspace tests and the root `mt`
check/build. Review the diff for accidental changes to ADR 0008, which may already
be dirty in the submodule worktree.
