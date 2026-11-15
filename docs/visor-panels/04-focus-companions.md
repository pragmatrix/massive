# 04 Focus companions

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [03](03-panels-on-the-visor.md).

## Goal

At the Focus zoom level, the camera centers the keyboard-focused view at 1:1 on screen. It also
shows the view that leads into it, when both fit. Text in the focused view is never blurry.

## Current situation

- The camera frames the keyboard-focused target
  ([ADR 0016](../adr/archive/0016-camera-zoom-is-relative-to-keyboard-focus.md)), at a zoom level relative
  to that target ([ADR 0018](../adr/archive/0018-zoom-is-a-level-relative-to-the-focused-target.md)).
- A view holds the keyboard focus itself, so a focused view is already a target the camera can
  frame ([ADR 0022](../adr/archive/0022-presenters-own-the-layout-of-their-targets.md)).
- At Focus today, the camera frames an instance's panel together with its title bar.

## Decisions

- **A view's focus companion is the view that Cmd+← would move to.** The desktop finds it by
  position, as navigation does ([03](03-panels-on-the-visor.md)). The companion may belong to
  another instance. The instance does not name companions.
- **A view has at most one companion.** Several companions are a follow-up.
- **1:1 always wins.** The camera frames both views at 1:1 when both fit on screen. Otherwise it
  frames the focused view alone, at 1:1. At Focus, the camera cancels the focused instance's
  instance scale, as it does today ([03](03-panels-on-the-visor.md)).
- **The camera aims at final positions.** The visor stage's placements are targets that apply at
  once ([08](08-animation-and-drawing.md)). So the camera moves straight to where the views will end
  up, while the views are still animating there.
- **Zoom levels further out are unchanged.** At Slot, Row, and Project, the camera behaves as today.

## Open questions

- Whether a companion is centered with the focused view, or whether the focused view stays centered
  and the companion only has to be visible.
- How the fit is measured when the companion's panel stands at an angle on the arc.
- What the camera does when the focused view alone is larger than the screen at 1:1.
- What the camera does when the companion belongs to an instance with a different instance scale,
  so that the two views can't both be shown at 1:1.
- How the zoom level indicator names Focus on a view.
