# 06 Visor layouter

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [05](05-layout-stage.md).

## Goal

The visor layouter is the layouter of the visor stage ([05](05-layout-stage.md)). It joins the
panels of a launcher's instances on the arc, sizes each panel, stacks its views, and slides each
panel vertically so that its remembered view sits on the center line. It is a pure function, so it
can be built and tested before the generic layout stage exists.

## What changed

The first session planned a column layouter that followed the layered graph drawing framework. It
derived each view's column from the connectors, ordered the views within a column to reduce
crossings, and split connectors into forward and backward ones. The application now supplies each
view's panel index and stack index itself, so these steps are gone.

The second session placed this layouter in the instance, as the panel layouter, and let it route the
connectors. [ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md) moved it into the
desktop as the visor layouter, and the connector stage now routes the connectors
([07](07-connector-routing.md)). Connectors influence the visor layouter only through the connector
lane counts it receives.

## Decisions

### Parameters

- **The desktop sets the visor layouter's sizes.** The minimum stack gap, the connector lane width
  and the minimum panel margins are tuning values. They live in the desktop's policy constants, like
  every other size picked by taste ([constants.rs](../../desktop/src/policy/constants.rs)).
- **The sliding strategy is an enum parameter of the visor layouter.** It is not a tuning value,
  because it chooses a behavior, not a size.
- **A view keeps its size.** The layouter never resizes a view. Views have different widths and
  heights.

### Inputs

- **The panels of the launcher's instances**, which `SyncChildren` gives the visor stage as its
  children, in the order of the instances, with the layout metadata of their views
  ([05](05-layout-stage.md)) and the views' sizes.
- **The layout focus** and the animation ticks, from the parent side.
- **The connector lane counts** of every stack gap and panel margin, from the connector stage above
  ([07](07-connector-routing.md)).

### Panels and stacks

- **Each instance's panels form a group.** The groups follow each other in the order of the
  instances. A nested instance's group goes after the parent instance's panel that the parent names.
- **The panel index orders the panels within a group.** Panels appear from left to right in the
  order of their indices.
- **The stack index orders the views of a panel.** Views are stacked from top to bottom in the order
  of their indices. Ties go to the view created first, so the order is always unambiguous.
- **A panel is as wide as its widest view, plus its panel margins.** Each panel margin is as wide as
  the connector lanes that the connector stage asks for in it. A narrower view sits centered in its
  panel.
- **Stack gaps keep their minimum size**, plus the connector lanes that the connector stage asks for
  in them.
- **Order changes animate.** When an index changes and views move, they move to their new places
  like after any other layout change.

### Remembered views and the center visor view

- **The layout focus sets the remembered view.** When a view gets the layout focus, it becomes the
  remembered view of its panel. Every other panel keeps its remembered view.
- **A closed view hands its place on.** When a remembered view closes, the view below it becomes
  remembered, or the view above it if it was the last one.
- **A panel's first view is remembered** when the panel appears.
- **The view that last held the layout focus in the launcher is its center visor view.** The
  layouter keeps it in its state and reports it to the parent side
  ([03](03-panels-on-the-visor.md)).

### Sliding

- **Every panel centers its remembered view on the center line.** The center line lies at y = 0.
  The focused view is the remembered view of its panel, so it sits on the center line too. When the
  layout focus moves within a panel, only that panel slides.
- **This is the only sliding strategy for now.** The strategy is a parameter, so that others can be
  added later.
- **A panel's vertical offset is set last.** The layouter first stacks a panel's views with their
  stack gaps, relative to the panel. Only then does it set the panel's offset from the center line.

### Outputs

- **Panels**, in order, each with its width, its height, its vertical offset from the center line,
  and its place on the arc, scaled by its instance's instance scale
  ([03](03-panels-on-the-visor.md)).
- **A title bar per panel**, placed as a node of the stage tree under its panel. It shows the title
  of the panel's remembered view, and its target is a part of that view
  ([03](03-panels-on-the-visor.md)).
- **Positions** for all views, relative to their panels.
- **The panel structure**, reported to the parent side for the connector stage. It is the order of
  the panels, the stack of views in each, the connectors, and the port rects, without sizes. A
  change to a port rect marks the visor stage again.
- **The remembered views**, reported to the parent side apart from the panel structure. A focus move
  then changes only which connectors are emphasized, not the route topology.
- **The center visor view**, reported to the parent side.

## Testing

Each rule above gets a test, because each can silently go wrong:

- the order of the groups, the panel order and the stack order, including the tie-break,
- the panel width with its margins, and the centering of narrower views,
- the stack gaps and margins growing by the connector lane counts they receive,
- the vertical offset that puts each remembered view on the center line,
- that a focus move within a panel slides only that panel,
- the handover of the remembered view when it closes,
- the first view of a new panel being remembered,
- the center visor view following the layout focus.

## Open questions

- The default values of the visor layouter's parameters.
- Whether an application may override some of them, for example the connector lane width.
- What a hole in the panel indices means, for example views with the indices 0 and 2 but none with
  1.
- Other sliding strategies, such as aligning a panel so that the views connected to the focused view
  sit beside it, and whether users can choose the strategy.
