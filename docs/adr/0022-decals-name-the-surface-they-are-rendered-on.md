# Decals name the surface they are rendered on

## Status: proposed

Today a **decal** is a visual with a global `decal_order`. That one number does two jobs. It sets the layer among all decals in the scene. It also ties a decal to the surface under it, but only implicitly, through the decal depth bias. The tie only holds while the surface has written depth before the decal is drawn.

Fading breaks the tie. Visuals with a location alpha below 1 are drawn after all opaque ones, back to front by eye distance, so the colors behind them are already in place. A decal sits at its own location, though, and that location's depth says nothing about the surface it is on. A terminal line is one example: it is offset from the panel origin. So the renderer currently draws all translucent decals after all translucent surfaces. Where two fading panels overlap, the front panel's depth then hides the back panel's text, which should show through faintly.

We decide that a decal names its surface. The renderer then draws each translucent surface with its decals, surface by surface.

## Model

- **Surface reference.** `Visual::on(surface, layer)` replaces `with_decal_order`. `surface` is a `Ref<Visual>`, so a surface outlives its decals. A decal arriving in the renderer whose surface is unknown is a broken invariant and is asserted.
- **Local layers.** `layer` orders decals only among the decals of the same surface. Unrelated UI no longer shares a number space: terminal text, launcher names and title bars stop competing for orders.
- **Free decals.** A decal without a surface (the zoom level indicator, the hover rectangle) keeps today's semantics: a global order, drawn after everything else. It is a distinct constructor so that "no surface" is a choice, not a forgotten argument.
- **Locations stay independent.** A decal's location need not be related to its surface's location. Only the drawing order is tied to the surface.

## Renderer

- `RenderBatches` indexes decals by surface id, then by layer. Free decals keep the global ordered map.
- **Opaque path:** unchanged and drawn pipeline by pipeline. Surfaces come first, then decals by layer across all surfaces, since the depth bias already resolves them.
- **Translucent path:** translucent surfaces are sorted back to front by clip-space `w`. Each one is drawn with its own pipelines, followed by its decals in layer order. A decal is drawn in the pass of its surface. Its own alpha is still applied, and alpha 0 still skips it.
- **Depth:** translucent surfaces keep writing depth. Drawn in back-to-front groups, a front surface only hides what was drawn before it, and its own decals still pass the bias test against it.

## Migration

1. **Scene API.** Add `Visual::on`, `VisualRenderObj::surface` and the free-decal constructor. Keep `with_decal_order` working as a free decal until all producers have moved.
2. **Renderer.** Add the surface index and the grouped translucent pass. Remove the separate translucent decal pass.
3. **Desktop producers.** The launcher name goes on the launcher background. The title bar text goes on the title bar background, which is in the same presenter. The zoom level indicator and the hover rectangle become free decals.
4. **Terminal.** The instance background visual belongs to the desktop, but the terminal creates its line, cursor and selection decals on its own runtime thread (ADR 0009). The instance context therefore hands the view a `Ref<Visual>` of its background surface next to its location. An instance rendered without a desktop background has no surface to hand over; such a view must submit its own surface visual.
5. **Remove `decal_order`.** It goes once all producers name a surface or are free decals.

## Considered Options

- **Infer the surface from the location tree**, for example as the nearest non-decal visual on an ancestor location. Rejected because the rule is implicit. The terminal's line locations and the desktop's background are not related through the tree, so the inference would be wrong exactly where it matters.
- **Translucent surfaces without depth writes, decals after all surfaces.** Rejected: where panels overlap, a back panel's text would draw at full strength over a fading front panel.
- **Group opacity** (render a fading subtree offscreen and composite it). This is exact for any nesting, but it needs render targets sized to the presentation scale and a new compositing path. It stays open if surface-local ordering turns out to be insufficient.
