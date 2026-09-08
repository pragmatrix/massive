# Camera zoom lives in a dolly distance, not a model scale

Supersedes the "morph feel" decision of ADR 0003. `PixelCamera` stored zoom as a uniform model `scale: f64` that multiplied the camera space; transitions lerped that model scale against the moving `look_at` anchor as two independent channels at the same eased `t`. Because the two channels are a product, a flat-band zoom-out (focus an instance, then the overview) traced a quadratic screen path and visibly swayed ~100px to one side and back instead of zooming monotonic. The camera's "zoom" is now a dolly distance along its view axis with the model scale removed; a quotient-of-`distance` projection is strictly monotonic, so the same transition never backtracks regardless of how the `look_at` anchor moves.

## Why distance instead of scale

For flat content the projection of a world point is `screen ∝ (scale · (p − anchor)) / cam_dist` under the old model — a product of two linearly-eased values (the model scale and the anchor offset), whose derivative can change sign. Moving magnification into the camera distance makes it `screen ∝ (p − anchor) / distance`, a Möbius quotient of the eased `distance` with a constant-sign derivative — monotonic by construction. The anchor's motion is absorbed into that quotient rather than multiplied against the zoom channel.

- Camera-space content (e.g. the focus-depth indicator) is not dollied: it stays at the fixed pixel-perfect distance so it remains on-screen regardless of world zoom.

## Fitting is direct, not a solver

Fitting content into the surface is an exact closed form. For flat content the camera looks straight at the focal plane: the on-screen scale is `cam_dist / distance`, so the distance that letterboxes a target of size `size` into the window is `cam_dist / fit_scale`. `fit_distance_for_points` generalizes this to depth-spanning sets by solving the on-screen constraint per point: a point at camera-space `(px, py, pz)` projects to `cam_dist * (px, py) / (d - pz_ndc)` at distance `d`, with the pixel depth converted into the dolly's NDC z units (`pz_ndc = pz / half_height` — the NDC transform scales all axes by 2/height); the binding distance is `d >= pz_ndc + cam_dist * |p| / half`. Points in front of the focal plane (`pz > 0`, closer to the camera) project larger and bind the fit; points behind it shrink and never bind. An empty point set returns `None` so the caller decides the fallback. No bisection solver (the previous `fit_scale_for_points` iterated 48 passes).

## Considered alternatives

- **Keep the model scale and just re-ease it.** Rejected: the two-channel product that causes the sway is inherent to scaling the model; re-easing alone cannot make the quotient monotonic.
- **Pin the `look_at` anchor to the focused panel so the anchor stops moving.** Rejected: the monotonicity of the distance quotient holds regardless of anchor motion, so this buys nothing and would de-center the overview on the group.
- **Depth-aware fit for depth-spanning sets (the 3D visor arc).** Initially deferred, now implemented in `fit_distance_for_points`: the per-point constraint with the pixel depth converted into the dolly's NDC z units (see "Fitting is direct, not a solver") is exact for the arc, not just flat content.

## Consequences

- Zoomed transitions are monotonic: edges slide to their target without the "right then back" backtrack, in both flat-band and visor cases.
- `scale: f64` is gone from `PixelCamera`; zooming live in `distance` (with the pixel-perfect distance `camera_distance(fovy)` as the identity zoom).
- Camera-space content stays pixel-fixed via `pixel_perfect_ndc_camera_move`, decoupled from world dolly.
- The 3D look-at capability (ADR 0003) is preserved: `look_at` still carries translate + rotate, and the overview still frames the projected silhouette of the visor points.
- Construction sites that used `fit_scale` (letterbox fit) now call `fit_letterbox_distance`; overview framing calls `fit_distance_for_points`.
