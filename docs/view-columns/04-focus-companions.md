# 04 Focus companions

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [03](03-multi-view-instances.md).

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

- **A view can name a focus companion.** While the view has the keyboard focus, the camera frames
  it together with its companion. The instance sends the companion as a view id, not as a rect,
  because the desktop already knows every view's box.
- **A view names at most one companion.** The column layouter names the selected view in the column
  to the left ([06](06-column-layouter.md)). Several companions are a follow-up.
- **1:1 always wins.** The camera frames both views at 1:1 when both fit on screen. Otherwise it
  frames the focused view alone, at 1:1. The camera's distance cancels the fit scale, as tested in
  [01](01-pixel-perfect-focus.md).
- **The camera aims at final positions.** The instance sends each view's final position once
  ([08](08-presentation-layer.md)). So the camera moves straight to where the views will end up,
  while the views are still animating there.
- **Zoom levels further out are unchanged.** At Slot, Row, and Project, the camera behaves as today,
  and the instance shows at its fit scale.

## Open questions

- Whether a companion is centered with the focused view, or whether the focused view stays centered
  and the companion only has to be visible.
- What the camera does when the focused view alone is larger than the screen at 1:1.
- How the zoom level indicator names Focus on a view.
