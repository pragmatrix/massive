# Per-instance shaping sessions

Supersedes the "Planned next — per-instance sessions" sketch in [ADR 0005](0005-dual-shaping-engines-cosmic-text-and-parley.md). That sketch proposed a replica/broadcast model for the font database; the design settled here is different and strictly less coordinated: fontique's *native shared collection* carries parley's font loading, and an *epoch-pull* pattern replaces broadcast for cosmic-text. Two grilling findings reshaped the sketch most: (1) font loading must be possible at any time — static-after-startup was rejected; (2) there must be exactly one shaping entry point — no instance/non-instance API split.

## Status: accepted; implementation phased (mt and desktop migrate together)

## Problem

The ADR 0005 published-registry amendment made the *render* path lock-free, but instances still serialize all shaping on the manager mutex: `FontManager` owns one engine instance (`FontManagerInner { engine: Box<dyn ShapingEngine>, face_metrics }`), and every `shaper()` session takes that one lock — even though shaping is per-task work whose inputs (the font database) are shareable and whose scratch contexts (parley's `FontContext`/`LayoutContext`, cosmic's `FontSystem` caches) are inherently per-thread state that the libraries themselves document as such. With several terminal instances shaping per frame, the mutex is the remaining shared resource in the shaping hot path.

## Design

### Sessions everywhere

`FontManager::shaper()` becomes `FontManager::session()`, returning a renamed `FontSession<'_>`. There is **no instance variant** of the API: instance, application, and desktop code all acquire sessions the same way. Sessions take `&self`: the pre-0006 reason for `&mut` gating everything — a session holding the manager's one mutex, where any other entry would re-enter it — is gone now that sessions shape on per-clone state and never touch the manager mutex. Exclusivity is enforced at runtime instead (below): `&mut` would have defended exactly one aliasing scenario that the session guard already serializes.

What changes is what the session locks. A session no longer holds the manager mutex for its whole duration; it borrows the *caller's* `FontManager` (which the caller owns per task) and the manager mutex stays free for other threads.

A session rule, made API-visible by this design: **a session must not outlive the frame cycle it shaped for** — its republish-on-drop is what keeps the renderer's contract exact (below).

### Session exclusivity at runtime, not via `&mut`

`session()` and `load_font` take `&self`. The `&mut` gate (ADR 0005's compile-time aliasing rejection) protected against exactly one hazard — two live sessions on one clone — and that hazard is now contained where the state lives: a session exclusively holds its clone's scratch mutex, and `session()` acquires it with `try_lock`, so a second session on the same clone **panics at the misuse point** (with a message naming the clone and the held scratch) instead of deadlocking on a non-reentrant parking_lot mutex. Sessions on different clones shape in parallel; `load_font` during an open session is safe by construction (the manager mutex is mint-time-only and never held while shaping). The remaining trade: a misuse that would once not compile now panics at runtime — judged worth it to drop the `&mut` plumbing through every presenter and constructor.

### Handles are detached, not cloned

`FontManager` does not implement `Clone`. Every handle's scratch is exclusively attached to
one logical owner; what was previously `clone()` is now an explicit `detached()`: the
returned handle shares only the mint authority (engine behind the mutex) and the published
snapshot, and gets a fresh, exclusively owned scratch seeded on first session. This makes
the instance-boundary split visible at every call site — a `detached()` call means "this
handle now shapes independently, contention-free" — instead of hiding inside a `Clone`
impl whose semantics a reader cannot see. `InstanceEnvironment` keeps its derived-shaped
`Clone` but detaches the font handle in its (manual) `Clone` impl: every spawned instance
is exactly the owner boundary ADR names.

### The manager stops owning shaping contexts

`FontManagerInner` keeps only the font-identity machinery; the engines' per-shape scratch moves out to per-task owners:

- **Registry mint**: the manager remains the *only* `FaceId` issuer (registration, lazy fallback interning). A `FaceId` is only meaningful within the manager that minted it — ADR 0005's consequence, now load-bearing across instances.
- **Published registry**: unchanged (`Arc<ArcSwap<FontRegistry>>`, the one `&mut`-gate exemption). Sessions republish on drop exactly as today.

### Parley: fontique's native shared collection

fontique 0.11 has built-in sharing: a `Collection` created with `CollectionOptions::shared = true` (or `make_shared()`) puts its data behind `Arc<Mutex<CommonData>>` with an internal version counter; every mutation bumps the version, every read (including `query`) syncs lazily on mismatch. `SourceCache::new_shared()` is the same pattern for font blobs.

Consequently there is **no replica, no broadcast, no registration registry** for parley:

- The manager's collection is in shared mode from construction (`system`/`bare`).
- Each per-task context is `FontContext { collection: shared.clone(), source_cache: shared.clone() }` — parley's fields are `pub`, no forking needed.
- `load_font` at any time registers into the shared collection and becomes visible to every instance on its next `query` — no manual invalidation, no lag beyond the in-flight layout.
- `FaceId`s derive from blob id + face index over the *one* shared collection, so they are intrinsically global and stable (the shared source cache's weak refs stay pinned by the published registry's strong refs, as today).

The canonical engine instance inside the manager becomes a *minter*: it runs `load_font`/registry-rebuild (including the symbol-fallback rebuild) and publishes; it no longer needs to be the per-frame shaping context of anyone.

### Cosmic: per-instance `FontSystem` with epoch-pull

cosmic's `fontdb` 0.23 has no shared mode (plain fields; `Database::clone` is a deep copy) and `FontSystem` privately owns its db, so fontique's trick is unavailable. Instead:

- Instances own a full `FontSystem` (its caches are per-instance anyway).
- The manager publishes, next to the registry, an **epoch** — the registry snapshot itself serves as the epoch token (swap = version bump).
- A cosmic session, at its start, compares the manager's published snapshot against the one it synced last; on mismatch it swaps its `FontSystem`'s db from the published snapshot and updates its local copy. New fonts are visible on the instance's next session — worst case one frame.
- Nobody keeps a list of live instances; the pull replaces broadcast.

Cost, accepted: a whole-`Database` clone per epoch change per instance (face metadata, not glyph data; bounded by installed fonts; epochs change only on actual loads). Interning of newly needed fallback faces routes through the manager (single `FaceId` authority) — a cold-path-only lock acquisition serialized by the manager mutex, so concurrent loads cannot conflict: register + publish + epoch bump are serialized; the registry is published atomically.

### Published metrics

`FaceMetricsCache` is deleted from `FontManagerInner`. Metrics are computed **eagerly at mint time** (under the already-held manager lock, at `load_font`/intern) and **published with the registry**: the snapshot carries `FaceId → (FontData, FaceMetrics)`. Per-shape consumers (the terminal's cluster-grid anchoring, ink checks) read metrics lock-free through `published()` exactly like the rasterizer reads font data. Values are instance-independent because faces are content-identical. Hot-path access drops from a manager-mutex acquisition to a snapshot `Arc` read.

### Renderer contract unchanged

The renderer's "registry miss is a real bug" stance stays absolute, and the debug assert of `GlyphRun.shaping_engine` against the manager's engine stays. Both survive because: republishing still happens at drop, sessions do not outlive their frame cycle, and drop precedes submission within a task.

## Considered options

- **Static-after-startup instances** (registry snapshot at construction). Rejected: font loading must be possible at any time; parley makes it free, cosmic via epoch-pull.
- **Replica/broadcast model** (per the ADR 0005 sketch). Rejected: it required a coordination structure of live instances precisely where fontique offers the same semantics built-in.
- **Epoch-pull for parley too.** Unnecessary: the shared collection already does version-checked sync internally.
- **Whole-db delta log instead of clone for cosmic pulls.** Rejected: reintroduces retention/replay bookkeeping to bound a cost that occurs at most once per load per instance.
- **Eager mint-time republish of interned cosmic faces** (publish inside `shape()`). Rejected on grilling review: a session is dropped before anything it shaped can be submitted, so the drop-publish window is unreachable; no contract change is needed.
- **Per-instance metrics caches.** Rejected in favor of publishing metrics with the registry — eager, identical across instances, and lock-free.
- **One session API with an instance/manager split.** Rejected: a single session API everywhere; ownership decides nothing about behavior.
- **Keeping the `&mut` gate** (`session()`/`load_font` as `&mut self`). Rejected once the session no longer holds the manager mutex: its only protection was against two sessions aliasing one clone — serialized by the scratch mutex anyway — while the `&mut` plumbing forced `&mut self.fonts` through every presenter and a clone-then-session dance in mt's `update_lines`. Replace it with the `try_lock` + panic guard (see Design).
- **Keeping derived `Clone` on `FontManager`.** Rejected: the clone's fresh-scratch
  semantics (the whole point — independent, contention-free shaping) were invisible at
  call sites, so any `.clone()` looked like a cheap share. `detached()` names the split.
  A session-keyed owner registry (claim scratch by task id) was considered to remove the
  derivation verb entirely; rejected until multiple sessions per owner over time are
  actually needed — it adds eviction and growth bookkeeping the current one-owner/one-scratch
  model does not have.

## Consequences

- Two live sessions on one handle panic loudly (`session()`'s `try_lock` guard) instead of compiling away — the runtime substitute for the former `&mut` gate; two *instances* shaping in parallel is the point — contention is gone because there is nothing shared to lock in the hot path.
- Handling a `FontManager` around is explicit: `detached()` marks every owner boundary;
  a derived `Clone` would have silently multiplied owners sharing nothing but identity.
- `load_font` mid-run is legal and becomes visible without coordinator knowledge: parley via fontique's version sync, cosmic via epoch-pull at next session start.
- Cosmic's per-instance `FontSystem` grows its caches without bound within one lifetime (its internals are not shareable by design) — the same cost every cosmic use pays; instances are long-lived, the cost is per-instance and bounded by workload.
- The manager's mutex is touched only by mint-time work (load, intern, publish); a benchmark gate (`benches/terminal_shaping.rs`) verifies the hot path no longer takes it.