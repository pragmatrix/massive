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
- The `shaping_engine` setting does not hot-swap; switching engines requires a restart.

## Amendment: no library-level default engine (2026-09-14)

There is no default shaping engine at the library level: `FontManager` has no `Default` impl, and
its constructor surface is `system(kind)` / `bare(kind)` — every client names its engine at
manager creation. `ShapingEngineKind::default_kind()` is gone. Defaults are a client decision:
mt's built-in config pins `shaping_engine "parley"` (per the benchmark evidence above), and
`DesktopEnvironment::new` requires the engine as a parameter so the shared font manager cannot
be built without one.