# Slots can be selected beside the keyboard focus

## Status: accepted, not implemented yet

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), whose slots and
placements it selects, and on [ADR 0021](./0021-keyboard-focus-state-lives-beside-the-router-not-in-it.md),
whose keyboard focus anchors the selection.

## Problem

Every configuration operation acts on exactly one slot, the focused one. Grouping (Cmd+G) wraps
only that slot into a new project, so gathering several neighboring slots into one project takes
a group, then a move for each further slot. The desktop has no way to say "these slots" instead of
"this slot".

## Decision

A **selection** is a rectangle of placements in one project's matrix. The selected slots are the
occupied placements inside it.

- **The anchor is the focused slot.** One corner of the rectangle is the placement of the slot
  that has the keyboard focus. The anchor is derived from the focus and never stored, so the two
  cannot disagree.
- **Only the far corner is stored.** The selection is a placement in the anchor's matrix, or none.
  The rectangle spans the anchor and the far corner in any direction. No selection means only the
  focused slot is meant, which is the behavior every operation has today.
- **The far corner moves placement by placement.** Shift+Cmd+arrow moves it by one column or one
  row, across empty placements too, so the rectangle grows and shrinks like a text selection. The
  keys repeat. The corner stays inside the matrix's columns and rows, and at an edge the key does
  nothing.
- **An instance anchors at its launcher.** When an instance has the focus, the first
  Shift+Cmd+arrow moves the focus to the instance's launcher, which then anchors the selection.
  A selection is about slots, and the keys an instance receives stay its own until the user
  extends.
- **Anything else collapses the selection.** A focus change, a zoom, or a configuration change
  clears it, unless the operation sets a new selection itself. A stored corner therefore never
  points into a matrix that has changed.
- **The selection lives beside the keyboard focus.** Like the keyboard focus (ADR 0021), it is a
  component that owns values only. Its operations return a change, which goes through the desktop's
  command and change pipeline (ADR 0002), so extending is a `DesktopCommand` that is planned into a
  change that sets the selection.
- **One outline covers the selection.** The focus outline expands to the rectangle of the selected
  placements, empty ones included, so the user sees the area the next operation acts on. Hover
  precedence over the outline stays as it is.

Operations that act on the focused slot will act on the selected slots instead. Which operations
do, and how, is decided per operation. Grouping and ungrouping are decided in
[ADR 0026](./0026-grouping-wraps-the-selected-slots-into-a-project-in-place.md).

## Consequences

- The matrix layout must keep its track geometry, the start and extent of every column and row.
  Today it only knows the rectangles of occupied placements, and an outline that spans empty
  placements needs the others too.
- A selection never crosses projects. Selecting slots of different matrices would need a rectangle
  that has no meaning in either.
- Shift+Cmd+arrow is taken at the desktop level, before an instance sees it, like the other
  Cmd+arrow shortcuts.

## Considered options

- **Move the corner slot by slot.** The corner would jump to the next occupied slot in the
  direction, and the outline could be built from known slot rectangles alone. Rejected, because the
  rectangle would grow in uneven steps over gaps, and the user could not tell from the keys alone
  how far the next press reaches.
- **Keep the focus in the instance while selecting.** The anchor would still be the launcher, but
  the instance would keep the keys. Rejected, because the focus outline and the selection outline
  would then sit on different targets, and the user could not tell which one the next key acts on.
- **Store a set of selected slots.** This would allow selections that are not rectangles, such as
  Cmd+click picks. Rejected for now, because every operation planned on a selection, grouping
  first, needs a rectangle to keep the slots' arrangement.
