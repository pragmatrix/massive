# 07 Connector routing

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [02](02-connector-shapes.md) and
[06](06-panel-layouter.md).

## Goal

Every connector gets a route from its source to its target that never crosses a view. Routes are
built only from the shapes in [02](02-connector-shapes.md), and every connector stays traceable.

## Decisions

### Where routes run

- **Connectors may join any two views.** That includes views in neighboring panels, views in panels
  further apart, views in a panel to the left of the source, and views in the same panel.
- **Routes never cross a view.** A route runs only through free space: the gaps between views
  stacked in a panel, the panels' margins beside their views, and the space above and below the
  stacks. The one exception is a connector's ends, which run horizontally inside their views, as
  described next.
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
- **There is no gap between panels.** Panels sit edge to edge on the arc
  ([03](03-multi-panel-instances.md)). All room for bends and vertical runs lies inside the panels,
  in their margins.
- **A connector to the next panel bends once, as one biarc.** The bend spans the source panel's
  right margin and the target panel's left margin. It crosses the edge between the two panels, and
  how it does so on the arc is open ([02](02-connector-shapes.md)).
- **Longer routes are lines and quarter arcs.** A connector that passes a panel, runs to a panel on
  the left, or stays in its panel is a chain of horizontal and vertical lines. Its corners and
  U-turns are quarter arcs.
- **A connector passes a panel through a gap or around its stack.** In every panel between its
  source and its target, it either passes through the gap between two stacked views, or it passes
  above or below the panel's stack.
- **A connector to a panel on the left turns around in the margins next to its ends.** It leaves its
  source on the right and turns in the source panel's right margin. It runs left past the panels in
  between and turns again in the target panel's left margin, to enter its target from the left. The
  standard layered framework calls these ends inverted ports.
- **A connector within one panel crosses it from right to left through a gap.** It leaves its source
  on the right, runs through a gap between stacked views from right to left, and enters its target
  on the left. Source and target are different views of the panel, so a gap between them always
  exists. The route therefore never goes around the panel's stack. The standard layered framework
  calls this an in-layer edge.

### Gaps, margins, and lanes

- **Gaps stay tight by default.** Where no connector passes, the gap between stacked views keeps its
  minimum size ([06](06-panel-layouter.md)).
- **Each connector gets a lane of its own.** A gap between stacked views grows by one lane for every
  connector passing through it.
- **Vertical runs take lanes in the panel margins.** A vertical run beside a view's right edge takes
  a lane in its panel's right margin, and one beside a left edge takes a lane in the left margin.
  The margin widens by one lane per run, which widens the panel ([06](06-panel-layouter.md)).
- **The lane width is a parameter of the panel layouter**, which the application sets
  ([06](06-panel-layouter.md)).
- **Lanes are ordered to cross as little as possible.** For several connectors leaving one view,
  this means that they nest. A connector that turns up from a higher point takes a lane closer to
  the view than one that turns up from a lower point, and the reverse holds for connectors that turn
  down. Ties go to the connector created first.

### Two phases

A panel is laid out relative to itself first. Widening a gap or a margin only changes the distances
within a panel. The panel's vertical offset is set last, by the sliding in
[06](06-panel-layouter.md), so a widened gap does not move anything in a fixed direction. If routes
were computed from absolute positions, every widened gap would change the positions they were
computed from. So routing runs in two phases.

1. **Topology.** The router decides which gap or which way around the stack each connector takes in
   every panel it passes. Then it assigns the vertical runs in every margin to lanes. None of this
   uses positions.
2. **Geometry.** The layout sizes every gap and margin for the lanes in it. It stacks each panel's
   views with these gaps, which fixes their positions relative to their panel. Then it sets each
   panel's offset by the sliding. Last, the routes get their exact lines and arcs.

The sliding changes the panels' offsets whenever the layout focus moves. Only the geometry phase
runs again then, so the routes keep their topology while the panels slide.

### Drawing order and emphasis

- **Connectors are drawn above the views.** Their ends reach into the views' margins, so they must
  show on top of the views.
- **The connectors of the remembered views are emphasized and drawn on top of the other
  connectors.**
- **Both ends show a filled circle** at the end's port, or in the view's margin without a port. It
  is a little larger than the stroke width and drawn above the line ([02](02-connector-shapes.md)).
- **Connectors do not react to the pointer.** Nothing about them is hit-tested.

## Testing

- A test against many generated layouts checks that no route crosses any view's box, apart from
  the horizontal runs between its own ports and their views' edges.
- Tests pin that a connector passing a panel through a gap routes through that gap, and one passing
  around the stack routes above or below it.
- Tests pin the turns of connectors to a panel on the left in the margins next to their ends.
- Tests pin lane ordering and its tie-break, including the nesting of several connectors that
  leave one view to the left without crossing each other.
- Tests pin that a gap or a margin grows by exactly one lane per connector.
- A test pins that a focus move changes only the geometry of the routes, not their topology.

## Prior art

- G. Sander, "A fast heuristic for hierarchical Manhattan layout", Graph Drawing, 1995 (published
  1996). Routing between layers with right angles, where segments in a gap are assigned to slots
  ordered to cross as little as possible.
- C. Schulze, M. Spönemann, R. von Hanxleden, "Drawing layered graphs with port constraints",
  Journal of Visual Languages and Computing, 2014. Ports on fixed sides, inverted ports for
  backward edges, and in-layer edges, as implemented in ELK.
- M. Wybrow, K. Marriott, P. J. Stuckey, "Orthogonal connector routing", Graph Drawing, 2009, as
  implemented in libavoid. It routes around fixed rectangles and then spreads overlapping segments
  apart. It does not fit here, because routing widens gaps and margins and so moves the views. Here
  the router fixes its topology before any position exists.

## Open questions

- How a connector crosses from one panel to the next on the arc ([02](02-connector-shapes.md)).
- How the router chooses the gap a connector passes through in a panel, or whether it goes around
  the stack. The first session let crossing minimization decide this, and that step is gone. The
  choice should not depend on positions, so that it survives the sliding.
- How far a connector runs above or below a stack when it passes around it.
- Which gap between source and target a connector within one panel crosses. The vertical distance
  it travels is the same for each of them, so lanes and crossings decide.
- How the radius of quarter arcs relates to the lane width.
- How lanes are assigned in the space above and below the stacks.
- **How a connector shows its direction.** A connector always leaves a view's right edge and enters a
  view's left edge, so its direction can be read from the sides. With a circle at both ends, this
  may be too subtle, above all for connectors to a panel on the left. The candidates are these.
  - No arrowheads. The sides alone show the direction.
  - An arrowhead at the target end, in place of the circle or next to it. The shape renderer has no
    triangle today, so this needs a new shape.
  - Different ends, such as a filled circle at the source and a ring at the target. A ring is four
    quarter arcs, so it needs no shape beyond the arc from [02](02-connector-shapes.md).

  The [demo](09-panels-demo.md) is the place to decide this, because only real layouts show
  whether the sides are enough.
