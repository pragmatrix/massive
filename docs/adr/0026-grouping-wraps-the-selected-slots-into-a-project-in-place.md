# Grouping wraps the selected slots into a project in place

## Status: accepted

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), whose `Group` and
`Ungroup` changes carry it out, and on [ADR 0025](./0025-slots-can-be-selected-beside-the-keyboard-focus.md),
whose selection says which slots are meant.

## Problem

A project is created empty, and the user then fills it slot by slot. When the launchers that
belong together already sit next to each other, the user has to rebuild their arrangement inside
the new project by hand, and the running instances must survive each move. The reverse is just as
tedious. Dissolving a project means moving each of its slots out and removing the project.

## Decision

**Grouping (Cmd+G) wraps the meant slots into a new project that takes their place.** Without a
selection, the meant slot is the focused one. With a selection, they are the selected slots. When
an instance has the focus, its launcher is meant.

- **The new project sits where the slots were.** It takes the top-left slot of the slots'
  bounding box in the parent's matrix. The bounding box spans the assigned slots only, so
  selected unassigned slots at the selection's border do not shift the result.
- **The slots keep their arrangement.** Inside the new project, each slot keeps its offset from the
  bounding box's top-left corner, which becomes the project's first column and first row.
- **The parent is not compacted.** The slots the grouped slots leave become unassigned, so nothing
  outside the selection moves.
- **Content moves, it is not recreated.** Each slot's content is re-parented, so its instances keep
  running and slide to their new places.
- **The new project takes the focus,** and the selection collapses.

**Ungrouping (Cmd+Shift+G) is the inverse.** It acts on the focused nested project, or on the project whose slots
are all selected (ADR 0029), never on the root.

- **The slots land where the project was.** The top-left corner of their bounding box lands on the
  project's slot, and they keep their arrangement. Ungrouping a project that grouping just
  created therefore gives back exactly what was grouped.
- **Slots in the way are pushed right.** Only rows in which a landing slot is assigned are
  touched. The slots in such a row move right just far enough to clear the landings, and
  unassigned slots between them absorb the push.
- **The ungrouped slots are selected.** The focus goes to the slot at a corner of their bounding
  box, the top-left corner first, and the selection's delta reaches the opposite corner. The
  user can then group them again, or narrow the selection first. When every corner of the
  bounding box is unassigned, only the focus moves and nothing is selected, because a selection
  has to start at the focused slot.
- **An empty project is removed.** It has no slots to move.

Grouping and ungrouping are each one change in the configuration (ADR 0011). The content and the
project need the same slot at the same time, which separate moves could only reach through a
temporary slot.

## Consequences

- A configuration change collapses the selection (ADR 0025) unless the operation sets a new one,
  as ungrouping does.
- The `Group` change takes several slots, and each of them is re-parented on its own.
- Grouping never moves anything outside the selection, but ungrouping can push slots of the
  parent. The asymmetry is deliberate. A project needs one slot, while its ungrouped slots can
  need many.

## Considered options

- **Place the new project at the selection's top-left corner, unassigned slots included.** Inner
  offsets would then count from the selection's border. Rejected, because ungrouping measures from
  the bounding box of the assigned slots, so a group followed by an ungroup would shift the slots.
- **Compact the parent after grouping.** The slots right of and below the selection would close
  the gaps it leaves. Rejected, because slots the user did not select would move, and ungrouping
  could no longer restore the arrangement.
- **Ungroup every selected project.** Rejected for now. It is unclear where the slots of several
  projects should land when their bounding boxes overlap after ungrouping.
