# Transforms name their pivot, not an anchor space

## Status: accepted, not implemented yet

## Problem

A `Transform` scales and rotates around a point, its pivot, and then moves the pivot to
`translate`. massive writes the same mapping with two different pivots. An origin-based transform
pivots at the local origin, the top-left corner of a rectangle. An anchor-based transform pivots at
another local point, which in massive is always the rectangle's center.

The functions that convert between the two are named `to_anchor_space` and `to_origin_space`, and
`SizedTransform` has a `to_origin_space` as well. The names suggest a change of coordinate space,
but the functions keep the mapping and only move the pivot. The word "anchor" is also taken by
unrelated concepts: the instance anchor that navigation lands on, the visor anchor sync, and the
camera's `look_at` anchor. A reader cannot tell from the call site what geometric operation happens.

## Decision

The conversions are named after the pivot they produce:

- `Transform::to_anchor_space(p)` becomes `Transform::pivoted_at(p)`. It returns the same mapping
  with `translate` as the image of the local point `p` instead of the origin.
- `Transform::to_origin_space(p)` becomes `Transform::pivoted_at_origin(p)`. It takes a transform
  pivoted at `p` and returns the same mapping pivoted at the origin.
- `SizedTransform::to_origin_space()` becomes `SizedTransform::pivoted_at_origin()`. Its
  documentation states that a `SizedTransform` is pivoted at the center of its rectangle.
- `DesktopLayoutState::layout_local_center` keeps its name. Its documentation states that it is the
  pivot of every layout placement.

The documentation of `Transform` defines the pivot once, and the conversion functions refer to it.

## Consequences

- The rename is mechanical but touches about 21 call sites in 12 files of the geometry and desktop
  crates. It lands as a commit of its own, apart from behavior changes.
- "Anchor" no longer names a transform concept, which leaves it to the navigation and camera
  meanings it already has.

## Considered options

- **One function that moves the pivot from one point to another.** A single
  `with_pivot_moved(from, to)` would cover both directions. Rejected, because every call site moves
  the pivot between the origin and the center, and two named directions read better there than two
  point arguments.
- **Keep the names and only improve their documentation.** Rejected, because the call sites are
  where the names are read, and "space" there still suggests a change of coordinates.
