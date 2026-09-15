# Parley shaping optimization plan (mt terminal loads)

Background for [ADR 0005](adr/0005-dual-shaping-engines-cosmic-text-and-parley.md):
the default parley engine trails cosmic-text on the per-cluster hot path; this
records the measured state and an ordered optimization list.

Measurement (criterion, `benches/hot_path.rs`, JetBrains Mono variable @ 13 px,
M-series Mac, 2026-09-15) shows parley now trails cosmic-text uniformly on the
terminal's per-cluster hot path:

| bench | cosmic-text | parley | ratio |
|---|---|---|---|
| `hotpath/repeat_same_cluster` (one cluster redrawn) | 813 ns | 1.89 µs | parley 2.3× slower |
| `hotpath/distinct_clusters` (512 new clusters, `find .` burst) | 406 µs | 939 µs | parley 2.3× slower |
| `hotpath/faceid_resolution` | 2.06 µs | 2.18 µs | ≈ par |

The gap is constant across cold (new text) and warm (repeated) loads, so it is
per-shape-call overhead, not a missing cache: cosmic-text's `shape-run-cache`
does not help its distinct case, and parley rebuilds everything per request.

## Where parley's per-shape time goes

`parley_engine::shape_line` (massive/shapes/src/shaping_engines/parley_engine.rs)
runs the full rich-text pipeline for every `ShapingRequest`, even a single-word
cluster:

1. A fresh `RangedBuilder` + resolved style tree + `Layout` per call, for text
   with at most one style span.
2. `break_all_lines(None)` — the full Unicode line-break algorithm (break table
   per char, word segmentation), although only line 0 is consumed.
3. `align(Alignment::Start, ..)` — a no-op reposition for Start alignment over
   one line, but still allocates and walks all lines/runs.
4. Two `line.items()` passes (counting pre-pass, then transfer) plus a
   `String`-cloning `FontFamilyName::Named` per request from
   `TextAttributes::named_family`.

Context and fallback resolution are already right (per-session shared-mode
collection clone, ADR 0006); fontique query and fallback are not the problem.
Per-face swash metrics are already cached (mint-time `FaceMetrics`), so the
historic per-cluster metrics cost is gone.

## Recommendations, in expected-payoff order

1. **Reuse a scratch `Layout` across shapes.** Use
   `RangedBuilder::build_into(&mut Layout)` (parley `builder.rs`) with one
   `Layout` per session instead of `builder.build(text)` allocating per call.
2. **Drop `align()` entirely.** The engine reads only line 0's metrics/glyphs;
   Start-aligned run offsets are already what `glyph_run.offset()` yields.
   Verify with `benches/glyph_run_counts.rs` (counts must stay identical).
3. **Instrument before deeper cuts.** Add bench variants isolating
   `build_into` / +`break_all_lines` / +`align` in `benches/hot_path.rs` so
   effort goes where the 1.9 µs actually is (builder resolve vs line breaker).
4. **Conditionally break.** For single-line terminal clusters, skip
   `break_all_lines` when a fast scan finds no breakable char class; this is
   where the Unicode break-table cost lives. Needs the instrumentation from 3.
5. **Kill per-cluster `String` clones.** Intern family names (fontique
   `FamilyId`) or map the terminal's default font to the allocation-free
   `TextFamily::Monospace` → `GenericFamily::Monospace`.
6. **Remove the counting pre-pass.** Walk `line.items()` once with modest
   `Vec` overallocation + truncate, or rely on amortized growth — negligible
   next to shaping.
7. **Run-level shape cache (secondary).** Key `(text, attrs, font_size)` →
   `ShapedRun` in `FontSession` (small LRU), matching cosmic's
   `shape-run-cache`. Only helps redraw/scroll of unchanged lines; the
   distinct-cluster floor only moves via 1–6. Engine-agnostic, benefits both
   engines.

Update this list with bench evidence (save criterion baselines via
`cargo bench --bench hot_path -- --save-baseline`) as each item lands; items
1, 2, 5, 6 are low-risk edits in `shape_line` alone.