# 08 Animation and drawing

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [02](02-connector-shapes.md),
[05](05-layout-stage.md), and [07](07-connector-routing.md).

## Goal

Panels, views, title bars and connectors move together and never drift apart. The desktop knows
where everything ends up at once, so that hit testing, navigation and the camera never wait for an
animation.

## What changed

The second session put a presentation layer into the instance's layout stage. It sent the targets
to the desktop once and animated only the drawing of the views, by offsetting each view's local
transform and easing the offset to zero. It also drew and animated the connectors on the same
clock. [ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md) moved the layout into the
desktop, so every placed node animates its own placement, and the connector stage draws the
connectors from the current values.

## Current situation

- The scene cannot animate on its own. Every moving transform is computed by a movement on the task
  side and sent again every frame (`massive-animation`,
  [ADR 0039](../adr/archive/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).
- The launcher's presenter animates its instances on the visor this way today
  ([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs)).
- The desktop hit-tests by its own placements, not by scene transforms
  ([hit_tester.rs](../../desktop/src/hit_tester.rs)).
- Each view has a local transform that the instance owns
  ([view.rs](../../applications/src/view.rs)).

## Decisions

- **Targets apply at once.** On each layout change, the visor stage's placements are targets. The
  desktop core places, hit-tests, navigates and frames the camera as if the panels and views were
  already there ([04](04-focus-companions.md)). The layout never runs per frame.
- **Every node animates its own placement.** A generic placed node with a scene location, a
  movement and a fade serves stages, panels, title bars and views
  ([05](05-layout-stage.md)). When its parent sets its placement, the node moves from its current
  value to the new one with the structural animation, whose duration and easing are desktop policy
  constants. This is how `set_placement(placement, animate)` works today.
- **Panels, views and title bars move together.** The visor stage sets all their placements in the
  same layout pass, and they share the structural animation's clock. A title bar never jumps ahead
  of its panel's views.
- **The instance's own transforms are left alone.** A placed node animates its own scene location.
  The local transform of a view stays the instance's.
- **Connectors follow the current placements.** While any placed node below a connector stage
  moves, it reports its current value every tick. The desktop core composes these values once into
  positions relative to the connector stage, and the connector stage recomputes the route geometry
  from them ([07](07-connector-routing.md)). So connectors and views can't drift apart.
- **A click during a move lands on the target.** While a view animates, it is hit-tested at its
  final position. A move takes as long as the desktop's structural animation.
- **Connectors are scene shapes.** They are rects, arcs, and endpoint circles from
  [02](02-connector-shapes.md), drawn by the connector stage above the views.
- **Emphasis follows the remembered views.** Connectors that leave a remembered view are emphasized
  and drawn on top of the others ([07](07-connector-routing.md)).
- **Connector colors are desktop policy colors**, and the stroke width and the diameter of the
  endpoint circles are desktop policy constants, like every other value picked by taste.

## Open questions

- How the scene draws the connectors above the views. The connector stage's location and the views'
  locations are different, and it is not yet checked what decides their order on screen, for
  example depth or submission order.
- How a route animates when its route topology changes, because connectors, views or panels were
  added or removed. Its pieces then differ before and after. The choices are to cross-fade the old
  and the new route, or to morph pieces where the structure matches and cross-fade otherwise.
- How a connector that crosses panel edges is drawn while the panels stand at angles on the arc
  ([02](02-connector-shapes.md)).
- How new views appear and closed views disappear, for example by fading.
- How the emphasis looks.
- Whether a scene that animates values on its own would remove the per-frame updates. This is a
  follow-up in the [README](README.md).
