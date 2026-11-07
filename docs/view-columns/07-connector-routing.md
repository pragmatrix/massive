# 07 Connector routing

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [02](02-connector-shapes.md) and
[06](06-column-layouter.md).

## Goal

Every connector gets a route from its source to its target that never crosses a view. Routes are
built only from the shapes in [02](02-connector-shapes.md), and every connector stays traceable.

## Decisions

### Where routes run

- **Connectors may join any two views.** That includes views in non-neighboring columns, backward
  connectors, and views in the same column.
- **Routes never cross a view.** A route runs only through free space: the gaps between columns, the
  gaps between views stacked in a column, and the space above and below the columns. The one
  exception is a connector's ends, which run horizontally inside their views, as described next.
- **A connector starts at a port and leaves it to the right.** Its endpoint circle sits at the
  middle of the source port's right edge. From there the line runs horizontally to the right, over
  the view, and leaves the view's right edge.
- **A connector ends at a port and arrives from the left.** It enters the target view's left edge
  and runs horizontally to the middle of the target port's left edge, where its endpoint circle
  sits.
- **Without a port, the end sits in the view's margin.** The endpoint circle sits at the view's
  vertical middle, a little inside the view's edge, at a distance that the application sets
  ([02](02-connector-shapes.md)).
- **The application decides what a connector covers.** The horizontal run between a port and the
  view's edge is drawn over the view's content. An application that wants its content clear of
  connectors places its ports in the margin.
- **A connector to the next column bends once, as one biarc.** The bend lies between the source
  view's right margin and the target view's left margin, and spans the gap between the columns.
  The horizontal runs from the ports lead into it.
- **Gaps between columns can be narrow.** A biarc's span includes both margins, and endpoints never
  sit in the gap. So the gap needs no room for endpoints, and the bend has more room than the gap
  alone. Vertical lanes still run in the gap itself, so a gap with lanes widens as before.
- **Longer routes are lines and quarter arcs.** A connector that passes a column, runs backward, or
  stays in its column is a chain of horizontal and vertical lines. Its corners and U-turns are
  quarter arcs.
- **A dummy node decides where a connector passes a column.** Every column that a connector passes
  holds a dummy node for it, ordered together with the column's views ([06](06-column-layouter.md)).
  A dummy node between two views makes the connector pass through the gap between them. A dummy node
  at either end of the column makes it pass around the column's top or bottom.
- **A backward connector turns around in the gaps next to its ends.** It leaves its source on the
  right, turns in the gap right of the source, runs left past the columns between, and turns again in
  the gap left of its target to enter from the left. The standard framework calls these ends
  inverted ports.
- **A connector within one column crosses it from right to left through a gap.** It leaves its
  source on the right, runs through a gap between stacked views from right to left, and enters its
  target on the left. Source and target are different views of the column, so a gap between them
  always exists. The route therefore never goes around the column's ends. The standard framework
  calls this an in-layer edge.

### Gaps and lanes

- **Gaps stay tight by default.** Where no connector passes, the gap between stacked views keeps its
  minimum size.
- **Each connector gets a lane of its own.** A dummy node takes the height of one lane in its
  column's stack, so a gap between stacked views grows by one lane for every connector passing
  through it. Gaps between columns grow the same way for vertical runs. The standard framework calls
  the lanes in a gap between layers routing slots.
- **Connectors stack horizontally in the gap between columns.** When several connectors of one view
  route to the left, each leaves the view's right edge and turns into a vertical lane of its own in
  the gap to the right of the view. These lanes sit side by side, so the gap widens by one lane per
  connector. The same holds for any vertical runs that share a gap between columns.
- **The lane width is a parameter of the column layouter**, which the application sets
  ([06](06-column-layouter.md)).
- **Lanes are ordered to cross as little as possible.** For several connectors leaving one view,
  this means that they nest. A connector that turns up from a higher point takes a lane closer to
  the view than one that turns up from a lower point, and the reverse holds for connectors that turn
  down. Ties go to the connector created first.

### Two phases

A column is laid out relative to itself first. Widening a gap only changes the distances between
the views of a column. The column's vertical position is set last, by the alignment in
[06](06-column-layouter.md), so the widened gap does not move anything in a fixed direction. If
routes were computed from absolute positions, every widened gap would change the positions they
were computed from. So routing runs in two phases.

1. **Topology.** Layering, dummy nodes, and crossing minimization ([06](06-column-layouter.md))
   decide which gaps between stacked views each connector passes through. Then the vertical runs in
   every gap between columns are assigned to lanes. None of this uses positions.
2. **Geometry.** The layout sizes every gap for the lanes in it. It stacks each column's views and
   dummy nodes with these gaps, which fixes their positions relative to their column. Then it sets
   each column's vertical position by the alignment. Last, the routes get their exact lines and
   arcs.

### Drawing order and emphasis

- **Connectors are drawn above the views.** Their ends reach into the views' margins, so they must
  show on top of the views.
- **The selected views' connectors are emphasized and drawn on top of the other connectors.**
- **Both ends show a filled circle** at the end's port, or in the view's margin without a port. It
  is a little larger than the stroke width and drawn above the line ([02](02-connector-shapes.md)).
- **Connectors do not react to the pointer.** Nothing about them is hit-tested.

## Testing

- A test against many generated layouts checks that no route crosses any view's box, apart from
  the horizontal runs between its own ports and their views' edges.
- Tests pin that a dummy node between two views routes through their gap, and one at a column's
  end routes around it.
- Tests pin the turns of backward connectors in the gaps next to their ends.
- Tests pin lane ordering and its tie-break, including the nesting of several connectors that
  leave one view to the left without crossing each other.
- Tests pin that a gap grows by exactly one lane per passing connector.

## Prior art

- G. Sander, "A fast heuristic for hierarchical Manhattan layout", Graph Drawing, 1995 (published
  1996). Routing between layers with right angles, where segments in a gap are assigned to slots
  ordered to cross as little as possible.
- C. Schulze, M. Spönemann, R. von Hanxleden, "Drawing layered graphs with port constraints",
  Journal of Visual Languages and Computing, 2014. Ports on fixed sides, inverted ports for
  backward edges, and in-layer edges, as implemented in ELK.
- M. Wybrow, K. Marriott, P. J. Stuckey, "Orthogonal connector routing", Graph Drawing, 2009, as
  implemented in libavoid. It routes around fixed rectangles and then spreads overlapping segments
  apart. It does not fit here, because routing widens gaps and so moves the views. Layered layout
  handles that with its dummy nodes.

## Open questions

- How crossing minimization weighs a detour around a column's end against crossings. The barycenter
  heuristic alone looks only at positions in the column to the left, not at route length.
- How far a backward connector runs above or below the columns when its dummy nodes sit at the
  columns' ends.
- Which gap between source and target a connector within one column crosses. The vertical
  distance it travels is the same for each of them, so lanes and crossings decide.
- How the radius of quarter arcs relates to the lane width.
- How lanes are assigned in the space above and below the columns.
- **How a connector shows its direction.** A connector always leaves a view's right edge and enters a
  view's left edge, so its direction can be read from the sides. With a circle at both ends, this
  may be too subtle, above all for backward connectors. The candidates are these.
  - No arrowheads. The sides alone show the direction.
  - An arrowhead at the target end, in place of the circle or next to it. The shape renderer has no
    triangle today, so this needs a new shape.
  - Different ends, such as a filled circle at the source and a ring at the target. A ring is four
    quarter arcs, so it needs no shape beyond the arc from [02](02-connector-shapes.md).

  The [demo](09-columns-demo.md) is the place to decide this, because only real layouts show
  whether the sides are enough.
