# The camera keeps the selection in view

## Status: accepted

Builds on [ADR 0025](./0025-slots-can-be-selected-beside-the-keyboard-focus.md), whose selection
the camera follows, and on [ADR 0018](./0018-zoom-is-a-level-relative-to-the-focused-target.md),
whose zoom level frames the focused target.

## Problem

The camera frames the keyboard-focused target at the zoom level. An extended selection reaches
beyond that target, so Shift+Cmd+arrow and Cmd+A can select slots the user cannot see, and the
next operation then acts on slots outside the view.

## Decision

**The camera keeps an extended selection in view, with a margin around it.** The margin is a
quarter of the slot extent on every side, along the same axis. It shows a strip of the neighboring slots, so the user sees where the selection ends. It is
measured in the coordinates of the selection's matrix, so a nested project scales it like its slots
(ADR 0015).

**The camera joins the zoom level's camera with the selection's camera.** The zoom level's camera
is resolved as before, from the focused target and the zoom level. The selection's camera frames the
selection with its margin, fitted into the window like a row or a project. The camera then faces
halfway between the two and frames the corners of both views, in 3D. When both face the same way and
one view already contains the other, this is the camera of the larger view, so the zoom level's
camera stays while it shows the selection's view. The view therefore only grows: what the zoom level framed stays in sight, and the
selection is added to it.

**The zoom level does not change.** A collapsed selection, or any zoom, therefore returns to the
zoom level's framing (ADR 0025 collapses the selection on every zoom).

## Consequences

- Setting the selection invalidates the camera, like a layout change does.
- The camera depends only on the focus, the zoom level, the selection and the layout, never on
  where it was before. Shrinking a selection back into the zoom level's framing returns the camera
  to that framing.
- A selection far from the focused target zooms out far, because the framing keeps the focused
  target in view as well.
- A camera's view is fitted to the window, so the selection's camera shows more than the selection
  with its margin along one axis. The zoom level's camera stays only when it shows that extra as
  well, and a joined view zooms out a little further than the selection alone would need.
- A joined camera facing halfway between two rotated framings matches neither of them. With an
  extended selection the focus sits on a launcher (ADR 0025), whose framing faces the matrix plane
  head-on like the selection's, so this does not happen today.

## Considered options

- **Center on the selection and never come closer.** The camera would slide to the selection
  at least at the zoom level's distance. Rejected, because what the zoom level framed could then
  leave the view, and the view would not only grow.
- **Test against the camera as it is.** The camera would stay put while a shrinking selection still
  fits, and so move less. Rejected, because the camera would then depend on its own history, and the
  same focus, zoom level and selection could end at different framings.
- **Join the zoom level's view with the selection and its margin.** The zoom level's camera would
  stay while it shows the selection with its margin, and the camera would otherwise frame its view
  together with that rect. It zooms out a little less. Rejected, because joining two cameras is one
  rule for any pair of framings, while this one treats the selection as a special case.
- **Test only the selection, frame it with the margin.** Rejected, because a selection that touches
  the edge of the view would count as visible while its margin lies outside, and the check and the
  framing would disagree about what must be seen.
