# Camera targets arbitrary 3D surfaces and point sets

The camera is no longer axis-aligned. `PixelCamera::look_at` is now built from a placement's full `Transform` (translate + rotate, scale forced to 1.0) instead of just its translation, allowing the camera to re-orient toward any flat 3D surface and move in depth (`z`). To make the camera interpolable, the old window-agnostic `CameraMode::Sized { target_size, blend }` (which resolved scale per render as a function of surface size) was replaced by a single resolved `scale: f64` (`1.0` = pixel-perfect). Camera transitions interpolate `look_at` and `scale` as one shared-eased curve (dolly-and-pan), and framing transforms points into camera space before fitting — so the camera can frame an arbitrary 3D point set, not just the focal plane. This is a generic camera capability; the rotated-visor follow is one specific application of it, not the requirement it was built for.

## Considered options

- **Straight-line focal-plane morph.** Rejected: the camera must rotate and move in depth, and under a projective transform you cannot pin any plane exactly — the reported "left edge goes right then back" curve is replaced by a natural dolly-and-pan instead.
- **Keep the camera axis-aligned and frame rotated content.** Rejected: framing content while never letting the camera re-orient toward it defeats the point of 3D surfaces; a rotated surface only makes sense if the camera can look at it head-on.
- **Overview camera stays axis-aligned.** Rejected: the overview frames a 3D point set, so it must transform points into camera space to fit them correctly.
- **Keep `look_at` scale from the placement.** Rejected: the camera shouldn't inherit content scale; zoom lives in the camera's resolved fit scale.
- **Keep `target_size` in the camera, resolve scale per render (previous design).** Rejected: scale stayed a function of surface size, so it could not be interpolated as a scalar, and the `blend` machinery it forced was the buggy, variant-heavy path we set out to remove (it caused a fullscreen-zoom snap-instant bug).
- **Keep a world rect and derive the interpolant another way.** Deferred, not chosen: it would keep the camera window-agnostic at the price of a more complex interpolation abstraction that nothing currently needs.

## Not in scope

- The straight-line focal-plane morph (abandoned: under rotation you cannot pin a plane exactly; the depth dolly is accepted as a deliberate effect).
- The visor depth/rotation layout itself (`visor_layout`, `visor_child_transform`) — that already produces the 3D placements; we only make the camera follow them.
- Glyph rasterization decisions: `GlyphClass` is derived from the rendered quad, not from `CameraMode`, so changing the camera representation does not affect glyph classification.

## Settled decisions

| Decision | Choice |
|---|---|
| Old parameter-based interpolation | Dropped — new method only |
| Focal-plane pinning | Abandoned — depth dolly accepted |
| 3D look-at | Live requirement |
| Meaning of "point at 3D" | Rotated / non-zero-z look-at |
| Focused visor orientation | Match the panel (`look_at = placement.transform`) |
| Inherit z too | Yes — full translate + rotate |
| Scope | Both camera follow + framing |
| `CameraMode` | Resolved `scale: f64` (drop `target_size`/`blend`) |
| `look_at` scale | Force 1.0 — zoom lives in fit scale |
| Overview rotation | Overview camera also inherits rotation |
| `CameraMode` → scale | Yes — thread `window_size` into construction |
| Morph feel | Dedicated shared-curve camera interpolation |
| Focused zoom | PixelPerfect (scale 1.0) |
| Shared curve shape | One eased `t` across translate/rotate/scale |
| Overview rotation source | Center/focused panel's yaw |
| Overview z | Inherit center panel's z |
| `points_fit_in_surface` | Rewrite to transform points by `look_at.inverse()` |

## Consequences

- The reported transition curve becomes a dolly-and-pan; depth dolly is a deliberate effect, not a defect.
- The camera can target any flat 3D surface head-on, or frame any 3D point set, because `look_at` carries orientation and depth and framing transforms points into camera space.
- One application is the rotated-visor follow: the overview camera inherits the focused panel's orientation and depth so zoom-out keeps that panel head-on while its siblings fan around it.
- `scale` conflates the user's content zoom with the fit-to-window letterboxing; two cameras built for different windows are now unequal.
- The desktop is the single, always-fresh constructor (always has `window_size` in hand), and a resize re-resolves the camera (`window_size_changed` → `camera_invalid()` → `resolve_desired_camera`), so the scale is never stale in practice.
- Every camera construction site now threads `window_size` through to resolve the fit scale at set-time; `Rect::to_camera` was removed (it could not resolve a scale without a surface size).
- The renderer no longer participates in window sizing for the camera path: `model_camera_matrix()` dropped its `surface_size` argument.
- The visor rotation itself remains gated behind `VISOR_ROTATION_ENABLED`; the camera capability is independent of that flag.
- Revisit if window-agnosticism becomes a real requirement (persistent camera state across resizes, or desktop-independent camera tests) — then the world rect must be preserved and the interpolant derived differently.

