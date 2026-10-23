# Matrix slots share one slot extent and center their content

## Status: accepted

Supersedes [ADR 0015](./0015-nested-projects-share-a-width-derived-presentation-scale.md).

Amended by [ADR 0034](./0034-a-project-header-does-not-widen-its-project.md): the header measures
no width, and the horizontal alignment of a stretched project's columns is a policy, leading for now.

Amended by [ADR 0035](./0035-sibling-projects-share-a-column-count.md): a nested project lays out the
columns of its widest sibling project and stretches only vertically.

## Problem

ADR 0015 scaled nested projects by width only and sized columns and rows by
their content. A tall project then made its row several slots tall, and a
project or a band made its column wider than the slots above and below it, so
columns no longer lined up across rows.

## Decision

Every slot's content fits one **slot extent** $E$: an instance extent, the
default panel with its title bar (ADR 0019).

- **Columns** are all $E_w$ wide, regardless of their content, including a
  column no slot reaches. **Rows** are as tall as their tallest presented slot,
  and since every slot fits $E$, never taller than $E_h$.
- **Content scales to fit**, like a full-screen instance fits the window
  (ADR 0014), and never enlarges. Sibling projects share one scale, so their
  relative sizes stay visible; for intrinsic project scenes $(w_i, h_i)$:

  $$
  s = \min\left(1, \frac{E_w}{\max_i w_i}, \frac{E_h}{\max_i h_i}\right)
  $$

  A launcher scales on its own: a visor or an empty launcher is exactly $E$ and
  stays at scale 1, a band shrinks to fit.
- **Expandable content stretches** to its intervals in layout space, extent $t / s$
  for an interval $t$, so the presented content fills its slot: a project along
  both axes, an empty launcher vertically.
- **Content is centered** on whole pixels. Each slot's content sits centered in
  its slot. In a stretched project, the header stays on top across the full
  width, and the matrix takes the remaining width and height and centers its
  intervals in it.
- **An empty project**, one without slots, measures its matrix as one slot
  extent, like an empty launcher, so it stays readable on its own.

## Layout dependency direction

Unchanged from ADR 0015. Measure project scenes bottom-up using cached child
measurements. A project's intrinsic scene includes its own children's
presentation policy but excludes the scale assigned by its parent. Compute the
parent's shared scale and matrix intervals from those measurements, then apply
placements top-down. Ancestor scales compose through transforms.

Placement must not feed allocated extents back into intrinsic measurement or
resize applications. A changed nested-project measurement must recompute the
parent's shared scale and sibling placements even when the parent's overall
measured size stays unchanged.

## Consequences

- One tall project shrinks every sibling project with it. ADR 0015 rejected
  height-based scaling for this reason; a matrix whose columns line up and
  whose rows stay within the default size is worth that cost.
- Bands and their instances render smaller in a matrix than in focus.

## Under discussion

The shared project scale. It equals the smallest scale any single sibling
needs, so one tall project shrinks a sibling that alone would fill its slot,
such as a project with a single launcher, to the same small scale. Per-project
fitting, rejected below, is the alternative being weighed against keeping the
siblings' relative sizes visible.

## Considered options

- Per-slot fitting for projects gives siblings different scales, obscuring
  their relative sizes.
- Fixing row heights as well as column widths would leave rows of short
  content, such as a project with one row of launchers, mostly empty.
- Letting a band overflow its column, as a visor's instances do, would overlap
  its neighbours with content that is not transient.
