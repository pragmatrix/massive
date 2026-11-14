# 06 Panel layouter

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [05](05-layout-stage.md).

## Goal

The panel layouter lays out a multi-panel instance's views in its panels. It sizes each panel,
stacks its views, and slides each panel vertically so that its remembered view sits on the center
line. It is a pure function, so it can be built and tested before the layout stage exists.

## What changed

The first session planned a column layouter that followed the layered graph drawing framework. It
derived each view's column from the connectors, ordered the views within a column to reduce
crossings, and split connectors into forward and backward ones. The application now supplies each
view's panel index and stack index itself, so these steps are gone. Connectors no longer influence
where a view goes. They decide only the routes and the room the routes need
([07](07-connector-routing.md)).

## Decisions

### Parameters

- **The application sets the layouter's parameters.** The minimum gap between stacked views, the
  lane width ([07](07-connector-routing.md)), the margins, and the sliding strategy are parameters
  of the panel layouter. The application passes them when it creates the stage, and the panel
  layouter supplies defaults. The desktop's policy constants do not apply, because the layouter
  belongs to the application, not to the desktop.
- **A view keeps its size.** The layouter never resizes a view. Views have different widths and
  heights.

### Panels and stacks

- **The panel index orders the panels.** Panels appear from left to right in the order of their
  indices.
- **The stack index orders the views of a panel.** Views are stacked from top to bottom in the order
  of their indices. Ties go to the view created first, so the order is always unambiguous.
- **A panel is as wide as its widest view, plus its margins.** The margins hold the connectors' runs
  and lanes ([07](07-connector-routing.md)). A narrower view sits centered in its panel.
- **Gaps between stacked views keep their minimum size** unless connectors pass through them
  ([07](07-connector-routing.md)).
- **Order changes animate.** When an index changes and views move, they move to their new places
  like after any other layout change.

### Remembered views

- **The layout focus sets the remembered view.** When a view gets the layout focus, it becomes the
  remembered view of its panel. Every other panel keeps its remembered view.
- **A closed view hands its place on.** When a remembered view closes, the view below it becomes
  remembered, or the view above it if it was the last one.
- **A panel's first view is remembered** when the panel appears.

### Sliding

- **Every panel centers its remembered view on the center line.** The center line lies at y = 0.
  The focused view is the remembered view of its panel, so it sits on the center line too. When the
  layout focus moves within a panel, only that panel slides.
- **This is the only sliding strategy for now.** The strategy is a parameter, so that others can be
  added later.
- **A panel's vertical offset is set last.** Gaps between views grow where connectors pass
  ([07](07-connector-routing.md)). The layouter first stacks a panel's views with their gaps,
  relative to the panel. Only then does it set the panel's offset from the center line.

### Outputs

- **Panels**, in the order of their indices, each with its width, its height, its vertical offset
  from the center line, and its remembered view.
- **Positions** for all views, relative to their panels.
- **Routes** for all connectors, from [07](07-connector-routing.md).

## Testing

Each rule above gets a test, because each can silently go wrong:

- the panel order and the stack order, including the tie-break,
- the panel width and the centering of narrower views,
- the vertical offset that puts each remembered view on the center line,
- that a focus move within a panel slides only that panel,
- the handover of the remembered view when it closes,
- the first view of a new panel being remembered.

## Open questions

- The default values of the panel layouter's parameters.
- What a gap in the panel indices means, for example views with the indices 0 and 2 but none with 1.
- Other sliding strategies, such as aligning a panel so that the views connected to the focused view
  sit beside it, and whether users can choose the strategy.
