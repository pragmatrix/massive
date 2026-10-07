# Nested projects share a width-derived presentation scale

## Status: accepted

Nested projects in a parent should preserve their relative visual sizes while
remaining readable. Each parent therefore applies one shared project scale to
all its direct nested projects, across every column, derived from the widest
nested project's intrinsic scene width and the preferred default panel width.

For preferred width $W_p$ and intrinsic nested-project sizes $(w_i, h_i)$:

$$
s = \min\left(1, \frac{W_p}{\max_i w_i}\right)
$$

Each nested project occupies its actual presented extent $(s w_i, s h_i)$.
Height does not constrain the scale: slots accommodate their scaled content
height. Projects never enlarge, and narrower projects retain their scaled
widths. Slots are left-aligned within their columns; column widths and row
heights are the maximum presented extents, with existing spacing preserved.
Launcher sizing and application viewport sizes are unchanged.

## Layout dependency direction

Measure project scenes bottom-up using cached child measurements. A project's
intrinsic scene includes its own children's presentation policy but excludes
the scale assigned by its parent. Compute the parent's shared scale and matrix
dimensions from those measurements, then apply placements top-down. Ancestor
scales compose through transforms.

Placement must not feed allocated extents back into intrinsic measurement or
resize applications. Placement consumes cached scene sizes rather than
recursively measuring subtrees. A changed nested-project measurement must
recompute the parent's shared scale and sibling placements even when the
parent's overall measured size stays unchanged.

## Considered options

- Per-slot fitting gives sibling projects different scales, obscuring their
  relative sizes.
- Constraining scale by height lets tall projects reduce every sibling's
  readability; dynamic slot heights preserve the width-based scale.
- Equal viewport widths with reflow require width allocation and subsequent
  height measurement. Shared presentation scaling preserves intrinsic layouts
  and keeps measurement independent of placement.