# Per-instance shaping

Supersedes the "Planned next — per-instance sessions" sketch in [ADR 0005](0005-dual-shaping-engines-cosmic-text-and-parley.md). That sketch proposed a replica/broadcast model for the font database; the design settled here is different and strictly less coordinated: fontique's *native shared collection* carries parley's font loading, and a *registry sync* pattern replaces broadcast for cosmic-text. Two grilling findings reshaped the sketch most: (1) font loading must be possible at any time — static-after-startup was rejected; (2) there must be exactly one shaping entry point — no instance/non-instance API split.

Terminology follows the glossary in [`CONTEXT.md`](../../CONTEXT.md): faces are **loaded** (client-directed) or **resolved** (picked up by the shaper at fallback time), the canonical engine behind the manager lock is the **face authority**, and per-session scratch alignment is **registry sync**.

## Status: accepted; implementation phased (mt and desktop migrate together)

## Problem

The ADR 0005 published-registry amendment made the *render* path lock-free, but instances still serialize all shaping on the manager mutex: `FontManager` owns one engine instance (`FontManagerInner { engine: Box<dyn ShapingEngine>, face_metrics }`), and every `shaper()` takes that one lock (the pre-0006 design) — even though shaping is per-task work whose inputs (the font database) are shareable and whose scratch contexts (parley's `FontContext`/`LayoutContext`, cosmic's `FontSystem` caches) are inherently per-thread state that the libraries themselves document as such. With several terminal instances shaping per frame, the mutex is the remaining shared resource in the shaping hot path.

## Design

### Shaping contexts everywhere

`FontManager::new_shaping_context()` creates a `ShapingContext`, and the context itself is the shaping session (ADR 0008 replaced an earlier borrowed-session design: `ShapingContext::shaper()` and a `ShapingSession` trait). There is **no instance variant** of the API: instance, application, and desktop code all shape through an explicit context. The context's session methods take `&mut self` — the authority mutex is registration-only and never held while shaping, so no compile-time borrow gate is needed on the manager. Exclusivity is a property of the context's ownership instead (below).

What a session holds: it does not hold the authority mutex for its duration; it uses the caller's `ShapingContext`, whose scratch is exclusive to that logical owner. Contexts sharing one manager state can shape concurrently.

A rule, made API-visible by this design: **a session must not outlive the frame cycle it shaped for** — the manager mutex is registration-only, so nothing about the session's lifetime needs to hold it; the rule instead defines when its shaped output stops being usable.

### Session exclusivity by exclusive scratch ownership

The context's session methods take `&mut self`, so the scratch stays exclusive to this context and two live sessions on one context are a **compile error**. Sessions on different contexts shape in parallel because their scratch is independent; `load_font` during an open session is safe by construction because the authority mutex is registration-only. `&mut self` exists only to hand out the scratch — it does not lock the manager, so a context is a per-owner value that a task holds exclusively rather than a shared handle.

### Contexts are explicit, managers are shared

`FontManager` does not implement `Clone` and owns no shaping scratch. It shares the face
authority and published registry through internal reference-counted state:
`state: Arc<FontManagerState>` with `FontManagerState { authority: Mutex<FontAuthority>,
published: Arc<PublishedRegistry> }`, and each `ShapingContext` holds that same `Arc` plus
its exclusive `Box<dyn EngineScratch>`. Each logical owner calls `new_shaping_context()` to
obtain fresh, reusable scratch. `InstanceEnvironment` shares the manager configuration
through `Arc<FontManager>`, while shaping owners retain their own contexts.

### The manager stops owning shaping contexts

`FontAuthority` keeps only the font-identity machinery; the engines' per-shape scratch moves out to per-context owners. The scratch itself is engine neutral: `ShapingEngine::new_scratch` creates a context's scratch, and an `EngineScratch` trait (`sync`, `shape`) drives it — the manager names no engine type after construction, and the per-engine seeding/sync strategies live entirely in each engine's scratch implementation.

- **Face authority**: the manager remains the *only* `FaceId` issuer (face loading and session-path resolution). A `FaceId` is only meaningful within the manager that registered it — ADR 0005's consequence, now load-bearing across instances.
- **Published registry**: a shared `Arc<PublishedRegistry>` bundles the engine kind with its
  `ArcSwap<FontRegistry>` under one `Arc`. The outer `Arc` is required — not incidental —
  because all contexts and renderers must observe the same swap cell, and bundling keeps
  `kind` and the snapshot from becoming mismatched parallel fields. Sessions publish
  resolved faces through the authority and refresh their snapshot after each shape.
- **Render-only registry source**: `FontRegistrySource` is a cheap handle over the same
  `Arc<PublishedRegistry>` for render-path reads (`engine_kind`, lock-free `registry`). It
  must not gain the face authority or a shaping scratch — it exists so the renderer never
  touches the manager or its mutex.

### Parley: fontique's native shared collection

fontique 0.11 has built-in sharing: a `Collection` created with `CollectionOptions::shared = true` (or `make_shared()`) puts its data behind `Arc<Mutex<CommonData>>` with an internal version counter; every mutation bumps the version, every read (including `query`) syncs lazily on mismatch. `SourceCache::new_shared()` is the same pattern for font blobs.

Consequently there is **no replica, no broadcast, no registration registry** for parley:

- The manager's collection is in shared mode from construction (`system`/`bare`).
- Each per-task context is `FontContext { collection: shared.clone(), source_cache: shared.clone() }` — parley's fields are `pub`, no forking needed.
- `load_font` at any time registers into the shared collection and becomes visible to every instance on its next `query` — no manual invalidation, no lag beyond the in-flight layout.
- `FaceId`s derive from blob id + face index over the *one* shared collection, so they are intrinsically global and stable (the shared source cache's weak refs stay pinned by the published registry's strong refs, as today).

The canonical engine instance inside the manager becomes the *face authority*: it runs `load_font`/registry-rebuild (including the symbol-fallback rebuild) and the session-path face resolution, and publishes; it no longer needs to be the per-frame shaping context of anyone.

### Cosmic: per-instance `FontSystem` with registry sync

cosmic's `fontdb` 0.23 has no shared mode (plain fields; `Database::clone` is a deep copy) and `FontSystem` privately owns its db, so fontique's trick is unavailable. Instead:

- Instances own a full `FontSystem` (its caches are per-instance anyway), seeded from the published snapshot only — a registry-only database means every session selection is a registry face by construction.
- Managers built with system fonts (`FontManager::system`) also carry a **prepared system database** as the fallback *candidate pool*: the canonical engine scans the full system font catalog exactly once, at construction, and every scratch seed afterwards builds its `FontSystem` over a clone of that database (a pure in-memory copy — no I/O, no re-parsing), registry faces loaded after it so loaded families win selection. A full catalog rescan per seed would cost a directory walk plus font-table parsing over the entire system fonts directory per manager handle — prohibitive, and unnecessary: the catalog only changes on OS font installation. A prepared database is also the reason the registry-only seed did not *have* to stay candidate-pool-free: the white-screen bug (system copies of loaded families) is avoided by loading registry faces last, not by removing the pool. `bare()` managers skip this entirely: no scan, no pool, hermetic registry-only sessions (tests rely on it).
- The manager publishes, next to the registry, a **sync token** — the registry's face count serves as the token (publication = version bump).
- A cosmic session, at its start, compares the manager's published face count against the one it synced last; on mismatch it loads the snapshot's unseen faces into its `FontSystem`'s db and updates its local count (registry sync). New fonts are visible to the instance on its next sync — worst case one shape. **Superseded: the sync runs before *every* shape, not once at session start.** A sync that lags a shape is not merely late: a mid-batch face is missing from the session database at selection time, and for a font reached only through fallback (nothing selects it by family) that means silently shaping glyph 0 of a face the session does know — see the ADR 0008 amendment (2026-09-27).
- Nobody keeps a list of live instances; the sync replaces broadcast.

Cost, accepted: incremental face replays per sync per instance (face metadata, not glyph data; bounded by known faces; registrations change only on actual loads). Resolution of newly hit fallback faces routes through the face authority (`resolve_face` under the manager mutex, published at resolution time) — a cold-path-only lock acquisition, so concurrent loads cannot conflict: register + publish are serialized and the registry is published atomically.

### Resolved faces publish at resolution time

Fallback faces the shaper picks (unknown scripts, emoji, symbols) are not loadable — they enter the identity world when a session first uses them: the scratch hands the face's data to the manager (`resolve_face`), which registers it under the manager lock and republishes immediately. Every published snapshot is therefore complete for every issued `FaceId`, and a frame submitted concurrently resolves the face lock-free. Within the same context, the context's captured snapshot is refreshed from the manager's latest publication at the end of every `shape` — so every face a returned run carries (its own fallbacks included) is visible to the context's `font_data`/`metrics` reads before the caller touches them.

### Published metrics

`FaceMetricsCache` is deleted from `FontManagerInner`. Metrics are computed **eagerly at registration time** (under the already-held manager lock, at `load_font`/resolution) and **published with the registry**: the snapshot carries `FaceId → (FontData, FaceMetrics)`. Per-shape consumers (the terminal's cluster-grid anchoring, ink checks) read metrics lock-free through `published()` exactly like the rasterizer reads font data. Values are instance-independent because faces are content-identical. Hot-path access drops from a manager-mutex acquisition to a snapshot `Arc` read.

### Renderer contract unchanged

The renderer's "registry miss is a real bug" stance stays absolute, and the debug assert of `GlyphRun.shaping_engine` against the manager's engine stays. Both survive because: publication happens at every registration (load and resolution), always under the manager lock, sessions do not outlive their frame cycle, and drop precedes submission within a task.

## Considered options

- **Static-after-startup instances** (registry snapshot at construction). Rejected: font loading must be possible at any time; parley makes it free, cosmic via registry sync.
- **Replica/broadcast model** (per the ADR 0005 sketch). Rejected: it required a coordination structure of live instances precisely where fontique offers the same semantics built-in.
- **Registry sync for parley too.** Unnecessary: the shared collection already does version-checked sync internally.
- **Whole-db delta log instead of clone for cosmic syncs.** Rejected: reintroduces retention/replay bookkeeping to bound a cost that occurs at most once per load per instance.
- **Eager registration-time republish of resolved cosmic faces** (publish inside `shape()`). Rejected on grilling review: a shaper is dropped before anything it shaped can be submitted, so the drop-publish window is unreachable; no contract change is needed.
- **Per-instance metrics caches.** Rejected in favor of publishing metrics with the registry — eager, identical across instances, and lock-free.
- **One shaper API with an instance/manager split** (instance vs. non-instance variants). Rejected: a single shaper API everywhere; ownership decides nothing about behavior.
- **Keeping the `&mut` gate on the manager** (`shaper()`/`load_font` as `&mut self` on
  `FontManager`). Rejected: shaping never holds the manager mutex, so the gate's only
  protection — two shapers aliasing one manager handle — is not a real sharing hazard,
  while the `&mut` plumbing forced `&mut self.fonts` through every presenter and a
  clone-then-shaper dance in mt's `update_lines`. The same exclusivity is expressed where
  it belongs, on the per-owner `ShapingContext` (`shaper(&mut self)`), leaving
  `FontManager` a shared `&self` handle.
- **A non-reentrant scratch mutex with a `try_lock` guard** (`shaper()` panicking on a second session). Rejected: the scratch is exclusive to its context and never shared, so the lock only existed to hand out `&mut` from a `&self`, and `&mut self` on `shaper()` rejects the same misuse at compile time while dropping `Mutex`/`MutexGuard` and the panic path from `ShapingContext`.
- **Keeping derived `Clone` on `FontManager`.** Rejected: the clone's fresh-scratch
  semantics (the whole point — independent, contention-free shaping) were invisible at
  call sites, so any `.clone()` looked like a cheap share. `new_shaping_context()` names
  the split.
  A shaper-keyed owner registry (claim scratch by task id) was considered to remove the
  derivation verb entirely; rejected until multiple shapers per owner over time are
  actually needed — it adds eviction and growth bookkeeping the current one-owner/one-scratch
  model does not have.

## Consequences

- Two live sessions on one context are a compile error (the session methods take `&mut self`); contexts
  sharing one manager state shape in parallel because their scratch is independent.
- Handling a `FontManager` around shares only identity and publication. Calling
  `new_shaping_context()` marks every shaping-owner boundary.
- `load_font` mid-run is legal and becomes visible without coordinator knowledge: parley via fontique's version sync, cosmic via registry sync at next sync.
- Cosmic's per-instance `FontSystem` grows its caches without bound within one lifetime (its internals are not shareable by design) — the same cost every cosmic use pays; instances are long-lived, the cost is per-instance and bounded by workload.
- The manager's mutex is touched only by registration work (load, resolve, publish); a benchmark gate (`benches/terminal_shaping.rs`) verifies the hot path no longer takes it.

## Implementation status and standing prohibitions (2026-09-29)

Implemented on `task-local-ui-contexts`/`master` (`d0a74e99` and neighbours): shared
`FontManagerState`, per-context scratch in `ShapingContext`, the `PublishedRegistry` bundle,
and the `FontAuthority` naming. Coherent session reads hold as designed: `shape` syncs
before every shape and — when the shape resolved a face — republishes the post-resolution
snapshot, so `font_data`/`metrics` resolve every face a returned run carries
(`shaped_faces_resolve_to_font_data`, `a_resolved_pool_face_is_readable_after_the_shape`).

The simplification plan that phrased this work (`docs/font-manager-simplification-plan.md`,
folded here) set the boundaries this design must not regress:

- **Do not make `FontManager` clonable.** `new_shaping_context()` makes the per-owner
  shaping boundary visible at call sites; a `Clone` would hide it again.
- **Do not replace snapshot publication with a mutex or a broadcast list.** One
  `ArcSwap` cell per publication, swapped under the authority lock, is what keeps renderer
  reads lock-free and lag-free.
- **Do not merge the face authority and the per-owner scratch back into one engine object.**
  The authority is registration work; the scratch is per-owner rendering cache. Merging
  them re-serializes shaping on the manager mutex — the problem this ADR exists to remove.

One cleanup remains deliberately open at the time of the fold: the engine contract still
carries `ShapingEngine::shape` and `ShapingEngine::font_data`, which production code no
longer calls — the canonical engine loads, publishes, seeds scratch, and resolves fallback
faces (`resolve_face`), and shaping runs through `EngineScratch::shape`. Removing them is a
follow-up: the `EngineScratch` callback seam is what session shaping actually consumes, and
the cosmic fallback-abort regression is covered through it (`cosmic_scratch.rs` tests) once
the trait methods go.