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

The `Ctrl+Cmd+F` View-menu action first enters native window fullscreen whenever
the window is windowed, regardless of focus. Once the native window is fullscreen,
the action toggles Full Screen Mode only when focus depth is `Instance` and
keyboard focus resolves to an instance: a base instance
toggles its launcher's mode, and an assistant toggles its own. In every other
focus state, the action exits native window fullscreen. `Cmd+Enter` starts an
instance like `Cmd+T`; starting an instance always commits `FocusDepth::Instance`
and focuses the new instance. Focus transitions, navigation, and window resize never
change Full Screen Mode.

## Considered options

Keeping the `InstanceFullScreen` depth as a camera Focus Depth was the rejected
alternative: it tied a presentation preference to camera zoom, forced reset
arms into every focus-transition path, and made fullscreen transient (lost on
focus change, resize, and restart) — the opposite of what the state means.
Routing the standard fullscreen menu action by focus depth keeps content scaling
separate from native window fullscreen while preserving `Cmd+Enter` for
starting an instance.

## Consequences

- `FocusDepth` loses its innermost variant; the depth indicator labels and the
  camera's fullscreen letterbox arm go with it.
- Starting an instance commits `FocusDepth::Instance`; stopping an instance and
  resizing the window leave focus depth unchanged.
- Zoom remains `Ctrl+Cmd+↑/↓`; starting instances is `Cmd+T` / `Cmd+Enter`,
  with `Shift` starting an assistant. Both keyboard shortcuts work on a focused
  launcher to start its first instance; the launcher presenter keeps click-to-start.