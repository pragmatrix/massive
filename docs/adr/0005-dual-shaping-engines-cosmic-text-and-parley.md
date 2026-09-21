# Dual shaping engines: parley (default) and cosmic-text

Text shaping is provided by two engines behind one capability contract in `massive-shapes`: the existing parley-based path and a cosmic-text-based path, both producing the same engine-neutral shaped-glyph data (`GlyphRun` and engine-neutral shaped glyphs) and sharing the swash rasterization path. The engine is selected at runtime when a `FontManager` is constructed — mt exposes this as the `shaping_engine` setting, applied at startup only. Parley is the default per the benchmark evidence below, and both engines compile in by default (cargo features `parley` and `cosmic-text` on `massive-shapes`; a build enabling neither fails to compile).

## First benchmark results (criterion, mt `benches/terminal_shaping.rs`)

Warm-cache shaping of the bundled JetBrains Mono at 13 px, M-series Mac:

- Single ASCII cluster (the per-cell hot path): cosmic-text 1.8 µs vs parley 2.1 µs — cosmic-text ~16% faster.
- Full mixed line (ASCII/CJK/emoji): parley 28 µs vs cosmic-text 137 µs — parley ~4.8× faster.
- Full-screen throughput (50 lines incl. lock): parley ~2× faster.
- Fallback stress: parley ~2.7× faster.
- Cold start (fresh font database per shape): parley 61 µs vs cosmic-text 36 ms — cosmic-text's `FontSystem` population dominates by ~600×.

Interpretation: cosmic-text's shaping-only path is lean for tiny inputs, but its per-shape font matching and lack of cross-shape caches lose badly on any multi-cluster line; the cold gap comes from `FontSystem` construction cost. The engine stays selectable at runtime so the terminal default can flip on evidence without recompiling; these numbers argue for parley as the terminal's default until the cosmic adapter gains line-batch shaping or a shared `FontSystem` reuse strategy.

## Considered options

- **Replace parley with cosmic-text outright.** Rejected for now: no measurement shows cosmic-text wins on typical terminal workloads. Parley remains the fallback engine; removing it is gated on the mt terminal-shaping benchmark results (criterion, both engines compared in one run across single-cluster, full-line, full-screen-throughput, and fallback-stress workloads).
- **Build-time-only engine selection (one engine per binary).** Rejected: honestly comparable benchmarks require both engines compiled into the same binary, and a runtime selection knob costs little once both compile.
- **Side-table mapping `fontdb::ID` → `FaceId` for the cosmic engine.** Rejected: font identity becomes an opaque `u64` newtype (`FaceId`) that each engine packs and unpacks engine-specifically — parley from blob id and face index, cosmic from `fontdb::ID`.
- **Tagging each `FaceId` with its engine kind.** Rejected: the tag was write-only in all live code — nothing ever branched on it; the one-engine-per-process selection (below) makes cross-engine id mixing impossible by construction, and a run-level debug stamp (see consequences) catches a stale run reaching the wrong renderer more directly.

## Consequences

- `FaceId` is no longer constructible from parley internals alone; a `FaceId` is only meaningful within the `FontManager` (engine instance) that produced it. Ownership is enforced by documentation and construction discipline, not runtime checks: a foreign id degrades to a graceful `font_data` miss.
- Runs carry their engine: each engine stamps `shaping_engine` onto its `ShapedRun` output (propagated to `GlyphRun`), and the renderer's text layer debug-asserts it against its manager's engine — a cheap tripwire if a run shaped by one engine is ever rendered through another engine's manager. Should engine-specific APIs ever return (e.g. parley contexts), they belong behind capability methods (`manager.parley_contexts() -> Option<_>`), not kind-gated shapers.
- `TextAttributes` becomes engine-neutral (family as data, not parley `FontFamily` values); parley-specific accessors such as `Shaper::contexts()` move behind the parley engine.
- Terminal cell-grid anchoring stays in mt and operates on engine-neutral shaped glyphs; the shaping contract stays "attributed text in, shaped glyphs out".
- The `shaping_engine` setting does not hot-swap; engine selection is fixed before the desktop
  starts.

## Amendment: no library-level default engine (2026-09-14)

There is no default shaping engine at the library level: `FontManager` has no `Default` impl, and
its constructor surface is `system(kind)` / `bare(kind)` — every client names its engine at
manager creation. `ShapingEngineKind::default_kind()` is gone. Defaults are a client decision:
mt's built-in config pins `shaping_engine "parley"` (per the benchmark evidence above), and
`DesktopEnvironment::new` requires the engine as a parameter so the shared font manager cannot
be built without one.

## Amendment: published font registry (2026-09-15)

The renderer's rasterization path resolved glyph faces through short `FontManager::shaper()`
sessions — acquiring the manager's one mutex per atlas miss, contending with instance threads
shaping under the same lock, for a read of data that is effectively immutable after startup. The
manager now *publishes* the registry instead: engines expose
`ShapingEngine::font_registry() -> Arc<FontRegistry>` (an immutable `FaceId → FontData` map), and
`FontManager::published(&self) -> Arc<FontRegistry>` hands the last-published snapshot to lock-free
readers. `TextLayerRenderer` reads it once per batch; the render path no longer touches the
manager mutex at all.

Republishing happens at every registry-mutating point, always under the manager lock:
`load_font`, and `Shaper` session end (`Drop`) — the cosmic engine lazily resolves fallback
faces during `shape()`, so a session may have grown the registry. Parley's registry is static
after startup (shape never resolves faces), so its snapshots never go stale mid-run.

The `&mut self` gate on `FontManager` entry points is unchanged — `published()` is the one
documented exemption (an `ArcSwap` load never touches the mutex, so it cannot deadlock against a
live session), and `FaceMetricsCache` stays out of the snapshot (per-manager, session-resolved).

Planned next — per-instance sessions. The renderer being lock-free removes only
*renderer-vs-shaper* contention; instances still serialize on the manager mutex because the
engines are inherently mutating during shaping (cosmic's `FontSystem` owns shaping caches and
interns on first use; Parley's contexts are single-use scratch). This follow-up has been
designed and accepted in [ADR 0006 — per-instance shaping sessions](0006-per-instance-shaping-sessions.md),
which supersedes the sketch that was here: sessions everywhere (one API, `shaper()` renamed
to `session()`), per-task shaping contexts, a fontique *shared-mode collection* for parley
(no broadcast needed — font loading is possible at any time), and an *epoch-pull* pattern
for cosmic's per-instance `FontSystem`. The manager keeps resolving fallback `FaceId`s and publishing
the registry; metrics join the published snapshot.

## Terminology (2026-09-15)

Later ADRs and the glossary in [`CONTEXT.md`](../../CONTEXT.md) use **resolved face** for what
this document calls *interned/minted* faces, and **registry sync** for what ADR 0006 called
*epoch-pull*. Terms here predate that refinement; read the older wording as the newer one.

## Amendment: the font policy belongs to the shell (2026-09-21)

**A font policy is the shell's construction input.** The engine and whether system fonts are
selectable are client decisions that live in client settings, and the shell must not read client
configuration. They therefore arrive as one `FontPolicy` parameter on `shell::run`, which is where
the rule this document states — no library-level default engine, every client names its engine — now
holds. The shell builds the font manager from the policy, constructs the application task's
[`TaskContext`](0008-task-local-ui-contexts.md) from it, and installs that context around the
application future. The desktop derives the manager from the context instead of constructing it, so
`DesktopEnvironment` no longer needs the engine in order to construct fonts.

**The policy is fixed at the task boundary.** A `FaceId` is only meaningful inside the manager that
issued it, so replacing a manager after a task has shaped would mix identities across contexts that
had already shaped; mutating a live manager's selection source has the same effect. The policy is
consumed by value at `shell::run` and the task's shaping context is installed before the application
starts, while nothing has shaped — a window the ownership model now enforces by construction, since
neither the application context nor the task context offers a way to replace the manager. Changing
the engine means a restart.

**Bare is a policy, not a default.** System fonts are a selection source the application never named,
and leaving that implicit has two costs that surface later. Cosmic scans the platform catalog when
its engine is built — the cold-start gap measured in the results above — so a terminal that loads
its own mono font still pays for a catalog it will not use. And an unnamed selectable face can win
selection: the shaper preferring the system copy of the terminal font over the loaded one is the
white-screen failure the fallback ordering in the cosmic engine guards against. The choice is stated
at the policy: `FontPolicy::bare` and `FontPolicy::system` are the two named policies, and the
manager-construction shorthands `FontManager::bare` and `FontManager::system` exist only to spell
them where a manager is built. Every manager is still built from a policy the caller passed, and a
caller that needs the flags separately uses `FontPolicy::new`.

Lazy system-font loading on a live manager was considered and rejected. System fonts are a selection
source, not registry faces: parley keeps them in a per-collection store that collection clones do not
share, and cosmic clones its candidate pool into each scratch. No shaper that already exists can
observe a later load without new per-engine re-seed machinery, and the registry's face-count sync
cannot cover a face that has no `FaceId` until something selects it — which nothing does while the
pool is empty. Naming the policy up front provides the same capability without that machinery.