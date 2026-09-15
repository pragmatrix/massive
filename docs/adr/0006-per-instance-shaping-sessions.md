# Per-instance shaping

Supersedes the "Planned next — per-instance sessions" sketch in [ADR 0005](0005-dual-shaping-engines-cosmic-text-and-parley.md). That sketch proposed a replica/broadcast model for the font database; the design settled here is different and strictly less coordinated: fontique's *native shared collection* carries parley's font loading, and a *registry sync* pattern replaces broadcast for cosmic-text. Two grilling findings reshaped the sketch most: (1) font loading must be possible at any time — static-after-startup was rejected; (2) there must be exactly one shaping entry point — no instance/non-instance API split.

Terminology follows the glossary in [`CONTEXT.md`](../../CONTEXT.md): faces are **loaded** (client-directed) or **resolved** (picked up by the shaper at fallback time), the canonical engine behind the manager lock is the **face authority**, and per-session scratch alignment is **registry sync**.

## Status: accepted; implementation phased (mt and desktop migrate together)

## Problem

The ADR 0005 published-registry amendment made the *render* path lock-free, but instances still serialize all shaping on the manager mutex: `FontManager` owns one engine instance (`FontManagerInner { engine: Box<dyn ShapingEngine>, face_metrics }`), and every `shaper()` takes that one lock (the pre-0006 design) — even though shaping is per-task work whose inputs (the font database) are shareable and whose scratch contexts (parley's `FontContext`/`LayoutContext`, cosmic's `FontSystem` caches) are inherently per-thread state that the libraries themselves document as such. With several terminal instances shaping per frame, the mutex is the remaining shared resource in the shaping hot path.

## Design

### Shapers everywhere

One shaper API: `FontManager::shaper()` returns a `Shaper<'_>`. There is **no instance variant** of the API: instance, application, and desktop code all acquire shapers the same way. Shapers take `&self` — the manager mutex is registration-only and never held while shaping, so no compile-time borrow gate is needed. Exclusivity is enforced at runtime instead (below): `&mut` would have defended exactly one aliasing scenario that the shaper guard already serializes.

What a shaper locks: it does not hold the manager mutex for its duration; it borrows the *caller's* `FontManager` (which the caller owns per task) and the manager mutex stays free for other threads.

A rule, made API-visible by this design: **a shaper must not outlive the frame cycle it shaped for** — the manager mutex is registration-only, so nothing about the shaper's lifetime needs to hold it; the rule instead defines when its shaped output stops being usable.

### Shaper exclusivity at runtime, not via borrow gating

`shaper()` and `load_font` take `&self`. A shaper exclusively holds its handle's scratch mutex, and `shaper()` acquires it with `try_lock`, so a second shaper on the same handle **panics at the misuse point** (with a message naming the handle and the held scratch) instead of deadlocking on a non-reentrant parking_lot mutex. Shapers on different handles shape in parallel; `load_font` during an open shaper is safe by construction (the manager mutex is registration-only and never held while shaping). The trade: a misuse that a compile-time gate would reject now panics at runtime — judged worth it to drop the `&mut` plumbing through every presenter and constructor.

### Handles are detached, not cloned

`FontManager` does not implement `Clone`. Every handle's scratch is exclusively attached to
one logical owner; deriving an independent handle is an explicit `detached()`: the
returned handle shares only the face authority (engine behind the mutex) and the published
snapshot, and gets a fresh, exclusively owned scratch seeded on first shaper. This makes
the instance-boundary split visible at every call site — a `detached()` call means "this
handle now shapes independently, contention-free" — instead of hiding inside a `Clone`
impl whose semantics a reader cannot see. `InstanceEnvironment` keeps its derived-shaped
`Clone` but detaches the font handle in its (manual) `Clone` impl: every spawned instance
is exactly the owner boundary ADR names.

### The manager stops owning shaping contexts

`FontManagerInner` keeps only the font-identity machinery; the engines' per-shape scratch moves out to per-task owners. The scratch itself is engine neutral: `ShapingEngine::new_scratch` creates a handle's scratch, and an `EngineScratch` trait (`sync`, `shape`) drives it — the manager names no engine type after construction, and the per-engine seeding/sync strategies (parley's shared-collection clone, cosmic's registry sync) live entirely in each engine's scratch implementation (`parley_scratch.rs`, `cosmic_scratch.rs`).

- **Face authority**: the manager remains the *only* `FaceId` issuer (face loading and session-path resolution). A `FaceId` is only meaningful within the manager that registered it — ADR 0005's consequence, now load-bearing across instances.
- **Published registry**: unchanged (`Arc<ArcSwap<FontRegistry>>`, the one `&mut`-gate exemption). Shapers republish on drop exactly as today.

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

- Instances own a full `FontSystem` (its caches are per-instance anyway), seeded from the published snapshot only — a registry-only database means every shaper selection is a registry face by construction.
- Managers built with system fonts (`FontManager::system`) also carry a **prepared system database** as the fallback *candidate pool*: the canonical engine scans the full system font catalog exactly once, at construction, and every scratch seed afterwards builds its `FontSystem` over a clone of that database (a pure in-memory copy — no I/O, no re-parsing), registry faces loaded after it so loaded families win selection. A full catalog rescan per seed would cost a directory walk plus font-table parsing over the entire system fonts directory per manager handle — prohibitive, and unnecessary: the catalog only changes on OS font installation. A prepared database is also the reason the registry-only seed did not *have* to stay candidate-pool-free: the white-screen bug (system copies of loaded families) is avoided by loading registry faces last, not by removing the pool. `bare()` managers skip this entirely: no scan, no pool, hermetic registry-only shapers (tests rely on it).
- The manager publishes, next to the registry, a **sync token** — the registry's face count serves as the token (publication = version bump).
- A cosmic shaper, at its start, compares the manager's published face count against the one it synced last; on mismatch it loads the snapshot's unseen faces into its `FontSystem`'s db and updates its local count (registry sync). New fonts are visible to the instance on its next shaper — worst case one frame.
- Nobody keeps a list of live instances; the sync replaces broadcast.

Cost, accepted: incremental face replays per sync per instance (face metadata, not glyph data; bounded by known faces; registrations change only on actual loads). Resolution of newly hit fallback faces routes through the face authority (`resolve_face` under the manager mutex, published at resolution time) — a cold-path-only lock acquisition, so concurrent loads cannot conflict: register + publish are serialized and the registry is published atomically.

### Resolved faces publish at resolution time

Fallback faces the shaper picks (unknown scripts, emoji, symbols) are not loadable — they enter the identity world when a shaper first uses them: the scratch hands the face's data to the manager (`Shaper` → `resolve_face`), which registers it under the manager lock and republishes immediately. Every published snapshot is therefore complete for every issued `FaceId`, and a frame submitted concurrently resolves the face lock-free. Within the same session, the session's captured snapshot is refreshed from the manager's latest publication at the end of every `shape` — so every face a returned run carries (its own fallbacks included) is visible to the session's `font_data`/`metrics` reads before the caller touches them.

### Published metrics

`FaceMetricsCache` is deleted from `FontManagerInner`. Metrics are computed **eagerly at registration time** (under the already-held manager lock, at `load_font`/resolution) and **published with the registry**: the snapshot carries `FaceId → (FontData, FaceMetrics)`. Per-shape consumers (the terminal's cluster-grid anchoring, ink checks) read metrics lock-free through `published()` exactly like the rasterizer reads font data. Values are instance-independent because faces are content-identical. Hot-path access drops from a manager-mutex acquisition to a snapshot `Arc` read.

### Renderer contract unchanged

The renderer's "registry miss is a real bug" stance stays absolute, and the debug assert of `GlyphRun.shaping_engine` against the manager's engine stays. Both survive because: publication happens at every registration (load and resolution), always under the manager lock, shapers do not outlive their frame cycle, and drop precedes submission within a task.

## Considered options

- **Static-after-startup instances** (registry snapshot at construction). Rejected: font loading must be possible at any time; parley makes it free, cosmic via registry sync.
- **Replica/broadcast model** (per the ADR 0005 sketch). Rejected: it required a coordination structure of live instances precisely where fontique offers the same semantics built-in.
- **Registry sync for parley too.** Unnecessary: the shared collection already does version-checked sync internally.
- **Whole-db delta log instead of clone for cosmic syncs.** Rejected: reintroduces retention/replay bookkeeping to bound a cost that occurs at most once per load per instance.
- **Eager registration-time republish of resolved cosmic faces** (publish inside `shape()`). Rejected on grilling review: a shaper is dropped before anything it shaped can be submitted, so the drop-publish window is unreachable; no contract change is needed.
- **Per-instance metrics caches.** Rejected in favor of publishing metrics with the registry — eager, identical across instances, and lock-free.
- **One shaper API with an instance/manager split** (instance vs. non-instance variants). Rejected: a single shaper API everywhere; ownership decides nothing about behavior.
- **Keeping the `&mut` gate** (`shaper()`/`load_font` as `&mut self`). Rejected: shaping
  never holds the manager mutex, so the gate's only protection — two shapers aliasing one
  handle — is serialized by the scratch mutex anyway; while the `&mut` plumbing forced
  `&mut self.fonts` through every presenter and a clone-then-shaper dance in mt's
  `update_lines`. The `try_lock` + panic guard (see Design) covers it.
- **Keeping derived `Clone` on `FontManager`.** Rejected: the clone's fresh-scratch
  semantics (the whole point — independent, contention-free shaping) were invisible at
  call sites, so any `.clone()` looked like a cheap share. `detached()` names the split.
  A shaper-keyed owner registry (claim scratch by task id) was considered to remove the
  derivation verb entirely; rejected until multiple shapers per owner over time are
  actually needed — it adds eviction and growth bookkeeping the current one-owner/one-scratch
  model does not have.

## Consequences

- Two live shapers on one handle panic loudly (`shaper()`'s `try_lock` guard); two *instances* shaping in parallel is the point — contention is gone because there is nothing shared to lock in the hot path.
- Handling a `FontManager` around is explicit: `detached()` marks every owner boundary;
  a derived `Clone` would have silently multiplied owners sharing nothing but identity.
- `load_font` mid-run is legal and becomes visible without coordinator knowledge: parley via fontique's version sync, cosmic via registry sync at next shaper open.
- Cosmic's per-instance `FontSystem` grows its caches without bound within one lifetime (its internals are not shareable by design) — the same cost every cosmic use pays; instances are long-lived, the cost is per-instance and bounded by workload.
- The manager's mutex is touched only by registration work (load, resolve, publish); a benchmark gate (`benches/terminal_shaping.rs`) verifies the hot path no longer takes it.