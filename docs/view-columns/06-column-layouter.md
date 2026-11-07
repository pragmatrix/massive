# 06 Column layouter

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [05](05-layout-stage.md).

## Goal

The column layouter places an instance's views in view columns. It derives the columns and their
order from the connectors, and it places each column next to the selected view on its left. It is a
pure function, so it can be built and tested before the layout stage exists.

## Approach

The column layouter follows the layered graph drawing framework, also called the Sugiyama framework
(see [Prior art](#prior-art)). In that framework, a graph's nodes are assigned to layers, and edges
run between layers. Here the nodes are views, the edges are connectors, and the layers are the view
columns.

The framework runs in steps, and this document names its decisions after them.

1. **Cycle removal** decides which connectors count as backward.
2. **Layering** assigns every view to a column, and inserts dummy nodes for connectors that pass a
   column.
3. **Crossing minimization** orders the views and dummy nodes within each column.
4. **Coordinate assignment** sets the vertical positions. This step differs from the standard
   framework, because it follows the selection.
5. **Edge routing** turns every connector into a route ([07](07-connector-routing.md)).

## Decisions

### Parameters

- **All columns have the same width.** The instance sets the width, and every view in a column takes
  it.
- **The application sets the layouter's parameters.** The column width, the gap between columns, the
  minimum gap between stacked views, and the lane width ([07](07-connector-routing.md)) are
  parameters of the column layouter. The application passes them when it creates the column layout,
  and the column layouter supplies defaults. The desktop's policy constants do not apply, because the
  layouter belongs to the application, not to the desktop.
- **A view keeps its size.** The layouter never resizes a view. Views have different heights.

### Cycle removal

- **Forward and backward connectors.** A connector is forward when its target view was created after
  its source view. Every other connector is backward. Creation order makes the split deterministic,
  even when connectors form a cycle.
- **Only forward connectors decide the columns.** Backward connectors are laid out like forward ones
  in the opposite direction, but they still leave their source on the right and enter their target
  on the left ([07](07-connector-routing.md)).

### Layering

- **A view's column comes from its forward connectors.** A view sits one column to the right of the
  furthest view that has a forward connector into it. A view without one sits in the first column.
  This is the longest-path layering.
- **A connector gets a dummy node in every column it passes.** A dummy node is a placeholder in its
  column's order. It stands for the connector passing that column, and it takes the height of one
  lane in the column's stack. A connector between neighboring columns passes no column and gets no
  dummy node. Backward connectors get dummy nodes in the columns between their ends, like forward
  ones.

### Crossing minimization

- **Views and dummy nodes are ordered together.** Each column is ordered by the average position of
  each entry's neighbors in the column to the left, sweeping from left to right. This is the
  barycenter heuristic. Ordering dummy nodes together with views keeps passing connectors from
  crossing connectors between views.
- **A dummy node's place decides where its connector passes.** Between two views, the connector
  passes through the gap between them. As the first or last entry, it passes around the column's top
  or bottom ([07](07-connector-routing.md)).
- **Ties go to the view or connector created first.** So the order is always unambiguous, and a
  small change to the connectors moves as little as possible.
- **Order changes animate.** When a connector changes and the order changes with it, the views move
  to their new places like after any other layout change.

### Coordinate assignment

The standard framework assigns coordinates globally, so that edges run as straight as possible
across the whole drawing. Here each column follows the selection instead, and the positions change
whenever the keyboard focus moves.

- **The first column starts at the top** of the content.
- **Every other column follows the selected view on its left.** The views that the selected view
  connects to are centered on where its connectors leave it. That is the selected view's box, or its
  port if the connector names one.
- **Without such connectors, middles line up.** If the selected view has no connector into the
  column, the column's middle lines up with the selected view's middle.
- **A column's vertical position is set last.** Gaps between views grow where connectors pass
  ([07](07-connector-routing.md)). The layouter first stacks a column's views and dummy nodes with
  their gaps, relative to the column. Only then does it set the column's vertical position by the
  alignment.

### Selection

- **The keyboard focus selects.** Focusing a view makes it the selected view of its column. Every
  other column keeps its selected view.
- **A closed view hands the selection on.** When a selected view closes, the view below it becomes
  selected, or the view above it if it was the last one.
- **A column's first view is selected** when it appears.

### Outputs

- **Positions** for all views, in the instance's content coordinates.
- **Routes** for all connectors, from [07](07-connector-routing.md).
- **The content extent.** It only grows while views exist and shrinks only when views close. So
  moving the focus never rescales the content.
- **Focus companions.** Each view's companion is the selected view in the column to its left
  ([04](04-focus-companions.md)).

## Implementation

The steps are written in Rust as part of the column layouter. No layout library is adopted.

- The layouts are small, so the standard heuristics are short to write.
- The coordinate assignment differs from every library's.
- The most complete implementation, ELK's Layered algorithm, runs on the JVM or in JavaScript.
- Rust crates for layered layout exist, but none has been evaluated. They are worth a look only if
  writing the steps turns out costly.

ELK serves as the reference for the standard steps, and as a comparison in tests.

## Testing

Each rule above gets a test, because each can silently go wrong:

- the forward and backward split, including cycles,
- the column of a view with several sources,
- the dummy nodes of a connector that passes several columns, and of a backward connector,
- the order within a column, including dummy nodes, and its tie-break,
- the vertical alignment, with and without connectors from the selected view,
- the content extent, which must not shrink when the focus moves,
- the handover of the selection when a selected view closes.

## Prior art

- K. Sugiyama, S. Tagawa, M. Toda, "Methods for visual understanding of hierarchical system
  structures", IEEE Transactions on Systems, Man, and Cybernetics, 1981. The framework itself.
- E. R. Gansner, E. Koutsofios, S. C. North, K.-P. Vo, "A technique for drawing directed graphs",
  IEEE Transactions on Software Engineering, 1993. The layered layout of Graphviz's `dot`, with
  dummy nodes and the median heuristic.
- U. Brandes, B. Köpf, "Fast and simple horizontal coordinate assignment", Graph Drawing, 2001. The
  standard coordinate assignment, which this layouter replaces with its own.
- The Eclipse Layout Kernel (ELK), algorithm "Layered". An open-source implementation of all steps,
  including port constraints and orthogonal routing.

## Open questions

- The default values of the column layouter's parameters.
- Whether one sweep from left to right is enough, or whether further sweeps in both directions are
  worth the less stable order they can produce.
- Barycenter or median heuristic. The median is often better on crossings, and the barycenter is
  smoother when connectors change.
- What happens when the alignment would push a column above the top of the content. The column could
  be clamped, or the whole content could shift down.
- Whether a view's column may jump when a new connector reaches it from further right, and how that
  animates.
