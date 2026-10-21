# Slots can be selected beside the keyboard focus

## Status: accepted

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), whose slots and
placements it selects, and on [ADR 0021](./0021-keyboard-focus-state-lives-beside-the-router-not-in-it.md),
whose keyboard focus the selection starts from.

## Problem

Every configuration operation acts on exactly one slot, the focused one. Grouping (Cmd+G) wraps
only that slot into a new project, so gathering several neighboring slots into one project takes
a group, then a move for each further slot. The desktop has no way to say "these slots" instead of
"this slot".

## Decision

A **selection** is a rectangle of placements in one project's matrix. The selected slots are the
occupied placements inside it.

- **The selection is a delta from the focused slot.** It is stored as the number of columns and
  rows it reaches from the focused slot's placement, negative towards the left and the top. The
  rectangle spans the focused slot's placement and the placement the delta reaches. The zero delta
  selects only the focused slot, which is the behavior every operation has today. The focused
  slot is never stored a second time, so the selection always starts where the focus is.
- **The delta grows placement by placement.** Shift+Cmd+arrow changes it by one column or one row,
  across empty placements too, so the rectangle grows and shrinks like a text selection. The keys
  repeat. The rectangle stays inside the matrix's columns and rows, and at an edge the key does
  nothing.
- **An instance selects from its launcher.** When an instance has the focus, the first
  Shift+Cmd+arrow moves the focus to the instance's launcher, where the selection then starts. A
  selection is about slots, and the keys an instance receives stay its own until the user
  extends.
- **Anything else collapses the selection.** A focus change, a zoom, or a configuration change
  resets the delta to zero, unless the operation sets a new selection itself. A stored delta
  therefore never reaches into a matrix that has changed.
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

- The matrix layout must expose its intervals, the start and extent of every column and row.
  The rectangles of occupied placements are not enough, because an outline that spans empty
  placements needs the others too. The outline derives the geometry with the same rule that places
  the slots, so the two cannot disagree.
- A selection never crosses projects. Selecting slots of different matrices would need a rectangle
  that has no meaning in either.
- Shift+Cmd+arrow is taken at the desktop level, before an instance sees it, like the other
  Cmd+arrow shortcuts.

## Considered options

- **Grow slot by slot.** The rectangle would jump to the next occupied slot in the direction, and
  the outline could be built from known slot rectangles alone. Rejected, because the rectangle
  would grow in uneven steps over gaps, and the user could not tell from the keys alone how far the
  next press reaches.
- **Keep the focus in the instance while selecting.** The selection would still start at the
  launcher, but the instance would keep the keys. Rejected, because the focus outline and the
  selection outline would then sit on different targets, and the user could not tell which one the
  next key acts on.
- **Store the far corner as a placement.** The selection would be the placement opposite the
  focused slot, or none. Rejected, because "none" and a corner on the focused slot would mean the
  same, and the stored placement would have to agree with the matrix of the focused slot, which
  the focus already determines.
- **Store a set of selected slots.** This would allow selections that are not rectangles, such as
  Cmd+click picks. Rejected for now, because every operation planned on a selection, grouping
  first, needs a rectangle to keep the slots' arrangement.
