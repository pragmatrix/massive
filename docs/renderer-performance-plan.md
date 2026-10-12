# Renderer performance plan

Status: **draft, not approved, postponed**. Written 2026-10-11 from a code reading;
nothing here is measured yet. Measurement comes first (phase 1), and every
optimization is gated by its numbers.

## Goal

Know where the renderer spends its time under realistic load, and remove the costs that
matter, in order of measured impact. The reference loads are terminal bursts (`cat` of a
large file) with 1 and 10 `mt` instances, and camera movement over a full desktop. The
out-of-process plan (`out-of-process-applications-plan.md`) measured the process
boundary at about 20 µs per full terminal screen; the renderer's per-frame work is
suspected to be larger, but nobody has measured it.

## Current situation

Change application is incremental:

- `Renderer::apply_changes` records changed visual ids.
- `prepare_batches` rebuilds batches only for those visuals.
- Unchanged visuals keep their GPU batches (`renderer/src/renderer.rs`).

Suspected costs, from reading the code:

1. **One new GPU vertex buffer per batch per changed visual.**
   - `AtlasRenderer::batch` calls `create_buffer_init`
     (`renderer/src/text_layer/atlas_renderer.rs`).
   - The quad and shape renderers do the same (`renderer/src/quads/renderer.rs`,
     `renderer/src/shape_renderer.rs`).
   - `mt` keeps one text visual and one overlay visual per line, so a burst that
     changes every line allocates ~120 buffers per terminal per frame.
2. **One new bind group per atlas batch,** although its inputs (atlas texture view and
   sampler) only change when the atlas grows.
   - Atlas growth replaces the texture and copies the old rectangles over
     (`renderer/src/glyph/glyph_atlas.rs`).
   - Batches built before growth keep a bind group that references the old texture,
     which therefore stays alive until every such batch is rebuilt. This is a suspected
     memory retention; verify it.
3. **Four CPU-built vertices per glyph.**
   - SDF glyphs use 4 × `TextureColorVertex` (36 bytes each), so 144 bytes per glyph
     are built and uploaded.
   - The index buffer must cover the largest batch (`prepare_index_buffer` scans every
     visual for `max_quads` each prepare).
4. **Atlas lookups per glyph:**
   - The SDF atlas is probed first, then the color atlas, then the empty-glyph set,
     each hashing a `RasterizedGlyphKey` that embeds the 32-byte `GlyphKey`.
   - Color glyphs (emoji) always pay two lookups.
5. **Per-frame draw work for every visible visual, changed or not:**
   - one draw call per visual per pipeline;
   - a push-constant update with the view-model matrix (a `Matrix4` product per
     visual per pipeline);
   - a bind group and a vertex buffer bind.
   - 10 terminals × ~120 visuals ≈ 1,200+ draws per frame before desktop chrome. The
     code already flags the repeated matrix work (`Architecture:` comment in
     `render_pipeline_batches`).

Existing measurement tooling:

- `MeasureSeries` (`renderer/src/stats.rs`) logs mean, min, and max render time when
  `RendererConfig::measure` is set. It waits for the GPU after submit, which distorts
  the frame pacing it measures.
- With the `metrics` feature, `massive_window_max_time_to_render` records the time
  from the oldest scene change to present (`shell/src/window_renderer.rs`), and is
  pushed to a Prometheus push gateway via `MASSIVE_METRICS_PUSHGATEWAY`
  (`shell/src/shell.rs`).
- `mt` has criterion benches for shaping and counting benches with committed
  baselines (`benches/`), but no renderer bench.

### PR #44 "Instanced rendering"

The PR was opened 2025-08-11 and last updated 2026-02-05: 4 commits, mostly generated,
touching `pods.rs`, `atlas_renderer.rs`, `text_layer/renderer.rs`, and both atlas
shaders.

- **Its design is what this plan wants:**
  - one instance per glyph, with `pos_lt`, `pos_rb`, `uv_lt`, `uv_rb`, color (SDF
    only), and depth: 52 bytes for SDF and 36 for color, instead of 144 and 96;
  - `VertexStepMode::Instance`;
  - an index buffer for a single quad, drawn with `0..6` indices and `0..count`
    instances;
  - `max_quads` and its per-prepare scan removed;
  - UV normalization stays in the shader, so atlas growth does not invalidate
    instance data.
- **It cannot be rebased.** It targets the pre-registry text layer (cosmic-text
  `FontSystem` behind a mutex, a per-layer `visuals` table, its own index buffer).
  Since then the renderer moved to `batch_producers`, `RenderBatches`, a shared quad
  index buffer, decals, clip rects, alpha, and the font registry snapshot.
- **The plan reimplements its design on the current code** (phase 3), uses its shader
  changes as reference, and closes the PR when that lands.

## Phase 1: measurement

### Instrumentation

All of it sits behind the existing `metrics` feature, so release builds without the
feature pay nothing.

- **CPU stage timings per frame** (histograms):
  - `apply_changes`;
  - `prepare_batches`, split into glyph lookup and rasterization, instance/vertex
    building, buffer creation, and bind group creation;
  - `prepare_index_buffer`;
  - `prepare_matrices`;
  - render pass encoding;
  - `queue.submit`;
  - `present`.
- **Counters per frame:**
  - changed visuals;
  - glyphs re-batched;
  - atlas misses and rasterizations;
  - buffers and bind groups created;
  - bytes uploaded;
  - draw calls and pipeline switches;
  - live atlas textures (verifies suspected cost 2).
- **GPU time:** wgpu timestamp queries around the main pass, when the adapter
  supports `TIMESTAMP_QUERY`, read back asynchronously. `MeasureSeries`' blocking
  wait is not used for these numbers.

### Scenarios

| Scenario | What it stresses |
|---|---|
| Idle desktop, 10 terminals | per-frame draw cost of unchanged visuals (cost 5) |
| `cat` burst, 1 terminal | per-changed-visual costs (1–4) |
| `cat` burst, 10 terminals | the same, at scale |
| Smooth scroll in one terminal | transform-only changes, matrix work |
| Camera zoom/pan over 10 terminals | matrix and draw cost with no visual changes |
| Emoji-heavy output (`just emoji`) | color atlas and double lookup (cost 4) |

Each scenario runs for a fixed duration in a release build on the M4 and reports the
p50/p99 of every stage timing and counter. The results are recorded in this document.

### Renderer bench

A headless criterion bench in `massive/renderer`. It renders to an offscreen texture,
with no window and no vsync, and feeds a synthetic scene change stream: full-screen
terminal frames generated like the IPC benchmark (200×60 glyphs, every line new) for 1
and 10 terminals, plus a transform-only stream. It measures `apply_changes` +
`prepare` + encode + submit, with a GPU wait, isolated from pacing.

The out-of-process plan's `scene::v1` wire format will make it possible to record real
`mt` change streams and replay them in this bench. Until then, the synthetic stream is
enough.

## Phase 2: cheap fixes (gated by phase 1 numbers)

- **O1. One bind group per atlas.**
  - The atlas owns its bind group and rebuilds it when it grows. Batches reference
    the atlas's current bind group at render time instead of owning one.
  - This removes a bind group creation per changed batch, and old atlas textures are
    freed right after growth.
  - Because rectangles keep their position on growth, existing instance data stays
    valid.
- **O2. A single atlas lookup.**
  - One map from `RasterizedGlyphKey` to `{ Sdf(rect, placement) | Color(rect,
    placement) | Empty }` replaces the two atlas probes and the empty-glyph set.
  - Color glyphs drop to one lookup; misses rasterize and insert once.

## Phase 3: instanced glyph quads (PR #44 design, reimplemented)

- Per-glyph instance records for SDF and color atlases as in PR #44, built directly in
  `TextLayerRenderer::runs_to_batches`. The atlas shaders expand the unit quad.
- The shared quad index buffer stays for the quad and shape pipelines. The atlas
  pipelines draw `0..6` indices with `0..count` instances.
- `max_quads` and the `prepare_index_buffer` scan go away for the atlas pipelines once
  no producer needs them.
- Validation: offscreen renders of the bench scenes before and after must be
  pixel-identical. That covers decals, clip rects, alpha, wide cells, and widened clip
  boxes.
- The same conversion for the quad and shape renderers is a follow-up, only if phase 1
  shows them in the profile.

## Phase 4: buffer arena (gated by phase 1 and 3 numbers)

- **O3. Replace per-batch `create_buffer_init`** with suballocation from a few large
  instance buffers:
  - written with `queue.write_buffer` at offsets;
  - with free ranges reused when visuals change or disappear;
  - growth by replacing the buffer and rewriting the live ranges.
- Batches then hold `(buffer, range)` instead of an owned buffer.
- It stays a separate phase because it changes batch ownership and lifetime, and
  instancing alone (4× smaller uploads) may already make allocations a minor cost.

## Phase 5: draw-call reduction (only if cost 5 dominates)

- **O4. Matrices once per location per frame.** Cache the `projection × location`
  product per location and frame instead of recomputing it per visual per pipeline.
- **O5. Fewer draws.**
  - Store per-location matrices, alpha, and clip rects in a storage buffer indexed
    per instance or per draw, so visuals sharing a pipeline can be drawn with one
    draw call (or multi-draw indirect where available).
  - This is a substantial rendering-architecture change (per-visual push constants
    become indexed data, and clip/alpha handling moves into the shaders) and gets its
    own plan if the idle and camera scenarios show draw submission as the bottleneck.

## Sequencing

1. Phase 1 instrumentation, scenarios, and the renderer bench; record the baseline.
2. Phase 2 (O1, O2) if their costs show up. O1 also lands if the atlas retention is
   confirmed, regardless of timing, because it is a memory leak in practice.
3. Phase 3, with pixel-identical validation; close PR #44.
4. Re-measure, then decide on phase 4 and phase 5 from the new numbers.

## Validation

- Bench and scenario numbers before and after each phase are recorded here.
- Pixel-identical offscreen renders for phase 3 and phase 4.
- Tests only where they pin a decision:
  - after atlas growth, batches render with the new texture and the old texture is
    released (O1);
  - the single lookup classifies a color glyph once (O2).

## Open questions

- Should the renderer bench live in `massive/renderer` (closest to the code) or in
  `mt`'s `benches/` (next to the existing baselines and real terminal data)? The plan
  assumes `massive/renderer`.
- Is `TIMESTAMP_QUERY` available on the target adapters (Metal on M-series, Vulkan on
  Linux)? Without it, GPU time comes only from the blocking-wait measurement in the
  bench, not from the live scenarios.

## Not part of this plan

Rasterization quality, the SDF pipeline itself, shaping (covered by `mt`'s shaping
benches), moving `prepare` off the render thread, and anything specific to out-of-process
applications.
