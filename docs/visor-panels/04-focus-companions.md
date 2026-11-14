# 04 Focus companions

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [03](03-multi-panel-instances.md).

## Goal

At the Focus zoom level, the camera centers the keyboard-focused view at 1:1 on screen. It also
shows the view that leads into it, when both fit. Text in the focused view is never blurry.

## Current situation

- The camera frames the keyboard-focused target
  ([ADR 0016](../adr/0016-camera-zoom-is-relative-to-keyboard-focus.md)), at a zoom level relative
  to that target ([ADR 0018](../adr/0018-zoom-is-a-level-relative-to-the-focused-target.md)).
- A view holds the keyboard focus itself, so a focused view is already a target the camera can
  frame ([ADR 0022](../adr/0022-presenters-own-the-layout-of-their-targets.md)).
- At Focus today, the camera frames an instance's panel together with its title bar.

## Decisions

- **A view's focus companion is the view that Cmd+← would move to.** The desktop finds it by
  position, as navigation does ([03](03-multi-panel-instances.md)). The stage does not name
  companions, so nothing crosses the boundary for them.
- **A view has at most one companion.** Several companions are a follow-up.
- **1:1 always wins.** The camera frames both views at 1:1 when both fit on screen. Otherwise it
  frames the focused view alone, at 1:1. An expanded multi-panel launcher shows its panels at 1:1,
  so the camera has no scale to cancel.
- **The camera aims at final positions.** The stage sends each view's final position once
  ([08](08-presentation-layer.md)). So the camera moves straight to where the views will end up,
  while the views are still animating there.
- **Zoom levels further out are unchanged.** At Slot, Row, and Project, the camera behaves as today.

## Open questions

- Whether a companion is centered with the focused view, or whether the focused view stays centered
  and the companion only has to be visible.
- How the fit is measured when the companion's panel stands at an angle on the arc.
- What the camera does when the focused view alone is larger than the screen at 1:1.
- How the zoom level indicator names Focus on a view.
