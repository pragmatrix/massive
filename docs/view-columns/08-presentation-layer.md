# 08 Presentation layer

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [02](02-connector-shapes.md),
[05](05-layout-stage.md), and [07](07-connector-routing.md).

## Goal

The presentation layer turns the layouter's results into what the user sees. It tells the desktop
where views are, draws the connectors, and animates views and connectors together, so that they
never drift apart.

## Current situation

- The scene cannot animate on its own. Every moving transform is computed by a movement on the task
  side and sent again every frame (`massive-animation`,
  [ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).
- The desktop hit-tests by its own placements, not by scene transforms
  ([hit_tester.rs](../../desktop/src/hit_tester.rs)).
- Each view has a local transform that the instance owns
  ([view.rs](../../applications/src/view.rs)).

## Decisions

- **Targets are sent once.** On each layout change, the presentation layer sends every moved view's
  new position to the desktop once. The desktop places, hit-tests, and navigates as if the view were
  already there. Its layout never runs per frame.
- **Only the drawing animates.** The presentation layer offsets each moved view's local transform by
  the distance it moved, then animates that offset to zero. This technique is known as FLIP: first,
  last, invert, play.
- **Views and connectors share one clock.** The presentation layer animates both, with the same
  start time, duration, and easing curve. That is why one layer owns both animations.
- **A click during a move lands on the target.** While a view animates, it is hit-tested at its
  final position. A move takes about as long as the desktop's structural animation.
- **Connectors are scene shapes.** They are rects, arcs, and endpoint circles from
  [02](02-connector-shapes.md), drawn under the location that the views hang from, above the views.
- **Emphasis follows the selection.** Connectors that leave a selected view are emphasized and drawn
  on top of the others ([07](07-connector-routing.md)).

## Open questions

- How the scene draws the connectors above the views. Connectors and views hang from the same
  location, and it is not yet checked what decides their order on screen, for example depth or
  submission order.

- **How connectors animate when their route changes shape.** Interpolating the pieces works only
  when the old and new routes consist of the same pieces. A route can gain or lose corners when
  views move. The choices are to cross-fade the old and the new route, to route again every frame
  against the animated positions, or to morph pieces where the structure matches and cross-fade
  otherwise.
- How new views appear and closed views disappear, for example by fading.
- How the emphasis looks. Connector colors, the stroke width, and the diameter of the endpoint
  circles are likely parameters that the application sets, like the layouter's parameters in
  [06](06-column-layouter.md).
- Whether a scene that animates values on its own would remove the per-frame updates. This is a
  follow-up in the [README](README.md).
