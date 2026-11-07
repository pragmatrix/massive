# 02 Connector shapes

Status: **decisions taken, experiment not started** (2026-11-07). Part of
[view columns](README.md).

## Goal

Connectors are drawn from a few primitive shapes that the shape renderer can draw sharply at any
scale and camera distance. A connector has one constant stroke width along its whole length.

## Current situation

- The shape renderer draws each shape as a quad. Its fragment shader computes an exact signed
  distance to the shape and smooths the edge with `fwidth`
  ([shape_renderer.wgsl](../../renderer/src/shape_renderer.wgsl)).
- A shape selector picks the distance function. Today there are rects, rounded rects, circles,
  ellipses, beveled rects, and stroked rects
  ([shape_renderer.rs](../../renderer/src/shape_renderer.rs)). Nothing draws a line or a curve.
- Each vertex carries the shape's size and two floats of shape data
  ([shape_renderer.wgsl:22](../../renderer/src/shape_renderer.wgsl#L22)).
- `sd_circle` already exists ([shape_renderer.wgsl:131](../../renderer/src/shape_renderer.wgsl#L131)).

## Decisions

- **Three shapes only.** Connectors are made of filled rects, angle-clipped arcs, and filled
  circles. Rects draw horizontal and vertical lines, and circles draw the endpoints. Both exist today
  (`ShapeSelector::Rect`, `ShapeSelector::Circle`). The arc is the one new shape.
- **The arc has an opening of up to 90°.** Its start lies on a horizontal or vertical tangent, so
  its start angle is a multiple of 90°. A quarter arc is the arc with an opening of exactly 90°.
  Half and three-quarter arcs are made of two or three quarter arcs.
- **The arc's distance is exact.** For a point p, a center c, a radius r, and a stroke width w, the
  stroke is where `|length(p − c) − r| ≤ w/2`. Outside the arc's angle range, the fragment is
  outside the shape. Clipping by angle cuts the stroke's ends along radii, which is at a right angle
  to the stroke.
- **S-bends are biarcs.** A bend between two horizontal lines at different heights is two mirrored
  arcs of the same radius. For a bend of horizontal span W and vertical drop H, with H ≤ W, the
  opening angle is `θ = 2·atan(H/W)` and the radius is `r = W / (2·sin θ)`. When the drop is larger
  than the span, the bend becomes a quarter arc, a vertical line of length H − W, and a quarter arc,
  with `r = W/2`. Below a drop of about half a pixel, the bend is drawn as a straight line, because
  the radius grows without bound.
- **Corners and U-turns are quarter arcs.** A turn from horizontal to vertical is one quarter arc. A
  U-turn is two.
- **Joins are hard cuts.** Every piece ends exactly where the next one starts, with the same
  tangent. A join is never a smoothed edge. Rects end at their quad's edge, and two quads that share
  an edge cover each pixel exactly once. Arcs end by their angle clipping, which is a hard test per
  pixel, not a smoothed one. So no pixel is covered twice or left out, and translucent connectors
  show neither darker spots nor lighter seams at joins. Two smoothed edges that meet would leave a
  faint lighter seam, because their partial coverages do not add up to full coverage when blended.
- **Endpoints are filled circles.** Both ends of a connector show a filled circle whose diameter is a
  little larger than the stroke width. The circle sits on the end's port, or in the view's margin
  when the end names no port ([07](07-connector-routing.md)). An endpoint can have a different color
  than its line.
- **Endpoints are layered above the line.** The line's first and last pieces run to the circle's
  center, and the circle is drawn on top of them. The circle hides the line's end, so the line needs
  no special end shape. The overlap is intended, because the circle covers the line there.

## Considered options

[biarc-vs-cubic.svg](biarc-vs-cubic.svg) shows the three curves for drops from 8 to 220 pixels
across a 120-pixel span.

- **Cubic Bézier curves with a numeric distance.** The exact distance to a cubic curve needs the
  roots of a fifth-degree polynomial per pixel. A numeric search with samples and Newton steps works
  for gentle curves, but is the least robust option for thick strokes on tight bends. Rejected.
- **Arc, straight diagonal line, arc.** This follows the cubic curve closely at every drop, with
  exact distances. It looked worse than biarcs for the small gaps between view columns. Rejected.
- **Biarcs.** They match the cubic curve for drops up to about half the span. Beyond that they turn
  into rounded right angles, which suits small gaps. Chosen.

For the endpoints, these options were considered.

- **A line end that stops at the circle's edge.** The flat end leaves small gaps, because the circle
  curves away from it. Rejected.
- **A line end with a concave cut that fits the circle.** It covers no pixel twice, but two smoothed
  edges meet along the circle and leave a faint seam. Rejected.
- **One combined shape, the union of the circle and the line's end.** It has no seam, but one shape
  has one color, and an endpoint can have a different color than its line. Rejected.
- **General union shapes.** A quad would refer to a list of primitives and draw their union. That
  joins any shapes without seams, but it needs a storage buffer for the list, and it has the same
  one-color limit. Not needed, because layering solves the endpoints.
- **Layering the circle above the line's end.** It uses existing shapes, allows two colors, and
  needs no new end shape. Chosen.

## Experiment

1. Add an angle-clipped arc selector to the shape shader and the shape types.
2. Draw biarc connectors from rects and arcs, with a translucent color, and endpoint circles of a
   second color above them.
3. Look at them at scales from 0.25 to 2, and at several camera distances.

## Done when

The joins show no seam and no overlap at any scale tried, and the edges are as smooth as the edges
of the existing rects.

## Open questions

- How the arc's orientation and opening are encoded. There are four start directions and two turning
  directions. The selector, the sign of a value, or the two floats of shape data could carry them.
- How large the quad around an arc must be. It needs the arc's bounding box, widened by w/2 and by
  the smoothing margin, on the sides where the stroke reaches.
- Whether `massive-shapes` names the arc after its geometry, such as `Arc`, or after its use.
