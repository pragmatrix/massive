# Full Screen Mode is per-launcher content scaling, not a zoom depth

## Status: accepted

The `InstanceFullScreen` Focus Depth is removed from the overview's Focus
Depths (`Project, Row, Slot, Instance`). Fullscreen-ness was a *camera* state
that coupled to keyboard focus: it lived in the global `focus_depth`, was reset
to `Instance` by focus transitions (instance start, stop, present) and by
window resize, and could only be reached by zooming onto the focused instance.
It is replaced by **Full Screen Mode**, a state of a kind of instance rather
than of the camera or the Focus Depths:

- **Per launcher, persisted.** The launcher stores one Full Screen Mode shared
  by all of its *base instances*. It rides the `Launcher` → `LaunchProfile` →
  `PersistedLauncher` chain and survives sessions in `desktop.json`.
- **Per assistant instance, temporary.** An *assistant instance* (started with
  `Shift+Cmd+T`, without the launcher's configured parameters) owns its own
  Full Screen Mode that never persists.

Full Screen Mode is *only* a content scale — analogous to the nested-project
slot scale: the launcher presenter and visor layout are untouched, placements
stay as they are, and nothing overlaps or reflows. Because the scale lives on
the instance transform, `camera_from_placement` resolves a focused full-screen
instance as pixel-perfect at that scale automatically; no camera or depth work
is involved.

`Cmd+Enter` toggles Full Screen Mode on whatever is focused: on a base
instance (or launcher) it toggles the launcher's mode, on an assistant instance
its own. It is a no-op on a launcher without instances. When the focus depth is
not already `Instance`, the toggle also re-commits `FocusDepth::Instance` — the
toggle zooms back to the instance level as a side effect. Nothing else ever
changes Full Screen Mode: not focus transitions, not navigation, not zoom, not
window resize.

## Considered options

Keeping the `InstanceFullScreen` depth as a camera Focus Depth was the rejected
alternative: it
tied a presentation preference to camera zoom, forced reset arms into every
focus-transition path, and made fullscreen transient (lost on focus change,
resize, and restart) — the opposite of what the state means. Repurposing
`Cmd+Enter` as the toggle reuses the key users already associate with
full-sizing the focused object and retires `Zoom::DefaultForFocused`, whose
only practical use was reaching the fullscreen depth.

## Consequences

- `FocusDepth` loses its innermost variant; the depth indicator labels and the
  camera's fullscreen letterbox arm go with it.
- The `CommitFocusDepth(FocusDepth::default())` resets on instance
  start/stop/present and the resize-exits-fullscreen arm are deleted.
- Zoom remains `Ctrl+Cmd+↑/↓`; starting instances is `Cmd+T` /
  `Shift+Cmd+T`. `Cmd+T` works on a focused launcher to start its first
  instance; the launcher presenter's `Cmd+Enter`-starts-instance branch is
  removed (the launcher presenter keeps click-to-start).