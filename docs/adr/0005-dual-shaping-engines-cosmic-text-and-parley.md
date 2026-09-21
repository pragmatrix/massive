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

## Amendment: shell-owned bare font manager and a replaceable font policy (2026-09-21)

The font manager is constructed in the shell as a *bare* manager and installed with the task
context; the desktop derives it from that context instead of constructing it. The rule this document
states — no library-level default engine, every client names its engine — is unchanged, but it now
holds at the shell boundary: the shell is given an engine kind and builds the manager from it, so
`DesktopEnvironment` no longer needs the engine in order to construct fonts.

**Bare by default.** System fonts are a selection source the application never named, and leaving
that implicit has two costs that surface later. Cosmic scans the platform catalog when its engine is
built — the cold-start gap measured in the results above — so a terminal that loads its own mono font
still pays for a catalog it will not use. And an unnamed selectable face can win selection: the shaper
preferring the system copy of the terminal font over the loaded one is the white-screen failure the
fallback ordering in the cosmic engine guards against. Bare by default makes every selectable font an
explicit application decision.

**A replaceable font policy.** The engine and whether system fonts are selectable are client
decisions that live in client settings, and the shell must not read application configuration.
Passing them as `shell::run` parameters alone would force either application config into the shell or
a default engine — and a default engine is what this document rejects.
`ApplicationContext::set_font_policy` instead lets a client replace the manager once, before the
desktop starts, which keeps font-content vocabulary out of the shell and removes a limitation
accepted below: changing the engine no longer requires a restart.

**Replacement rather than mutation.** A `FaceId` is only meaningful inside the manager that issued
it, so mutating a live manager's selection source would mix identities across contexts that had
already shaped. Replacing the manager makes the invalidation total and explicit, confines it to the
window where nothing has shaped yet, and reuses the existing `system()` constructors — no engine
changes. Ownership enforces that window: the switch is a method on the context the desktop consumes.

Lazy system-font loading on a live manager was considered and rejected. System fonts are a selection
source, not registry faces: parley keeps them in a per-collection store that collection clones do not
share, and cosmic clones its candidate pool into each scratch. No shaper that already exists can
observe a later load without new per-engine re-seed machinery, and the registry's face-count sync
cannot cover a face that has no `FaceId` until something selects it — which nothing does while the
pool is empty. Replacing the manager provides the same capability without that machinery.