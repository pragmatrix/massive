# The selection surrounds the focused slot

## Status: accepted

Amends [ADR 0025](./0025-slots-can-be-selected-beside-the-keyboard-focus.md): its decision that
the selection is a single delta from the focused slot is replaced. Everything else in ADR 0025 stays
in force.

## Problem

Cmd+A should select all slots of a project. Under ADR 0025, a selection spans the focused slot and
the one slot its delta reaches, so the focused slot is always a corner of the selection. Selecting
all slots therefore has to move the focus to a corner of their bounding box. That corner may be
unassigned on every side, for example when four slots form a diamond, and then no selection covers
them all.

Spreadsheets, the closest comparable applications, keep the active cell where it is and select the
area around it. Text editors move the caret to the end, which works for them because text has only
two ends.

## Decision

**A selection is two offsets from the focused slot.** One counts the columns and rows the
selection reaches to the left and above the focused slot, the other the columns and rows it reaches
to the right and below. Both are counts, so neither can point the wrong way, and unlike ADR 0025's
delta they are not displacements. The focused slot lies anywhere inside the rectangle. The zero
offsets select only the focused slot, as before.

**Shift+Cmd+arrow shrinks the opposite side first.** A step to the right moves the selection's left
edge to the right while the selection reaches left of the focused slot, and moves its right edge
otherwise. The same holds for each direction. A selection that reaches to one side only behaves
exactly as under ADR 0025.

**Cmd+A selects the bounding box of all assigned slots of the focused slot's project.** The project
whose matrix holds the focused slot consumes the key on its way down the keyboard focus path
(ADR 0020) while a launcher or a nested project has the focus. The focus stays where it is. When
an instance has the focus, the key passes on to its application, which uses Cmd+A for its own
"select all", as Terminal.app, iTerm2 and Ghostty do. When all slots are already selected, Cmd+A
changes nothing.

**Esc and Cmd+Esc collapse an extended selection.** The desktop takes them only while the selection
reaches beyond the focused slot, and keeps the focus where it is. An instance never has the focus
while a selection is extended, because extending hands it to the launcher and Cmd+A starts only
from a launcher or a project, so both keys always reach an application that has the focus. Without an extended selection, both keys escape the
focused target by one level ([ADR 0031](./0031-esc-escapes-the-focused-target-by-one-level.md)).

## Consequences

- Selecting all slots from inside an instance takes a step out first, to the launcher. The
  terminal client drops Cmd+A until it binds it to select all of its text.
- Ungrouping still focuses a corner of the ungrouped slots and selects to the opposite one
  (ADR 0026). It could now keep the focus elsewhere, but that is left for a later decision.
- A selection made with Cmd+A shrinks towards the focused slot from whichever side the arrow points
  away from.

## Considered options

- **Keep one delta and move the focus to a corner.** Rejected, because a project whose bounding box
  has no assigned corner could not be fully selected, and the focus would jump.
- **Store a separate "all slots" state beside the delta.** Rejected, because the outline, grouping
  and ungrouping would each have to handle a second kind of selection.
- **Grow first, then shrink.** Shift+Cmd+arrow would move the edge in the arrow's direction until it
  reaches the matrix's edge. Rejected, because a step in one direction would no longer undo a step
  in the other.
