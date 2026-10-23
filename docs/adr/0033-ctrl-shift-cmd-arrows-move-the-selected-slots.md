# Ctrl+Shift+Cmd+arrow moves the selected slots

## Status: accepted

Builds on [ADR 0025](./0025-slots-can-be-selected-beside-the-keyboard-focus.md) and
[ADR 0030](./0030-the-selection-surrounds-the-focused-slot.md), whose selection it moves, and on
[ADR 0024](./0024-configuration-operations-are-one-command-vocabulary.md), whose command vocabulary
it extends.

Amended by [ADR 0035](./0035-sibling-projects-share-a-column-count.md): the block moves within the
columns its project shares with its sibling projects, not only within its assigned slots.

## Problem

Rearranging a project's matrix from the keyboard is not possible. The configuration commands
`MoveSlot`, `SwapSlots`, and `PushSlot` exist, but only the CLI (`mt move`, `mt push`) reaches
them, and each acts on one slot. Moving a group of neighboring slots takes one CLI call per slot,
in the right order.

## Decision

**Moving is a selection operation.** Ctrl+Shift+Cmd+arrow moves the selected slots one slot in the
arrow's direction. Without an extended selection it moves the focused slot alone, because the zero
offsets select only that slot. The keys repeat, like the other Cmd+arrow shortcuts.

**The selection moves as one rigid block.** All slots inside the selection rectangle keep their
arrangement, assigned and unassigned ones alike. A selected nested project moves like a launcher.

**The content in front of the block takes the place the block leaves.** The slots the selection
newly covers form a strip as wide or as tall as the selection, directly in front of it. Their
assigned content moves to the strip the block leaves behind, each to the slot in the same row for a
horizontal move or in the same column for a vertical one. A move is therefore never blocked by
content, and no slot outside the strip in front changes. A move in one direction is undone by a
move in the opposite one. For a single slot this is the swap with an assigned neighbor that
`mt move` already does. Text editors move selected lines the same way (VS Code with Opt+↑/↓,
JetBrains IDEs with Opt+Shift+↑/↓): the line in front jumps behind the block.

**The block stays inside the matrix as it is before the move.** The matrix spans from its
top-left slot to the rightmost column and the lowest row that hold content. At any of its edges, a
move towards that edge does nothing, so a move never grows the matrix, and the selection stays
inside the matrix, as extending already keeps it (ADR 0025). Content in the last column or row
cannot move further right or down for now. Growing the matrix by a move is left for a later
decision, which also has to say whether empty slots at the selection's edge may leave the
matrix.

**A move never crosses a project.** The block stays in the matrix of its project. It neither leaves
a nested project at its edge nor enters a neighboring one, just as a selection never crosses
projects (ADR 0025).

**The focus and the selection move with the block.** The focused slot moves by one, and the
selection keeps its offsets, so the next press moves the same block again.

**Each kind of configuration change decides whether the selection survives it.** ADR 0025
collapses the selection on every configuration change. Now only the changes that add, remove, or
move slots collapse it: assigning, clearing, moving, swapping, grouping, and ungrouping. A swap
collapses it, because the CLI's `mt move` may swap the focused launcher away, and the offsets
around it could then reach past the matrix's edge. The block move keeps the selection, because the
focused slot moves with the block. A change that leaves every matrix as it was, such as setting the
startup launcher, keeps it too. The decision is made per kind of change and not per project,
because the CLI runs in a terminal that need not have the focus, and the rule should stay as
simple as the list of changes.

**A focused instance moves its launcher.** The instance keeps the focus. An instance never has an
extended selection (ADR 0030), so only the launcher's slot moves.

**The move is one command.** A new configuration command, `MoveSlots`, names the project, the
selection's rectangle, and the direction. It takes the rectangle and not only the assigned slots
in it, because the strip in front spans the whole rectangle, its unassigned slots included. The desktop system plans it into configuration changes
in one place, together with the new focus and selection (ADR 0024), so the block moves in one step
and never passes through a half-moved state.

## Consequences

- Zoom In and Zoom Out (Ctrl+Cmd+↑ and Ctrl+Cmd+↓) must match exactly their modifier set. Today
  they match any modifier set that contains Ctrl and Cmd, so Ctrl+Shift+Cmd+↑ would zoom instead of
  moving.
- Ctrl+Shift+Cmd+arrow is taken at the desktop level, before an instance sees it, like the other
  Cmd+arrow shortcuts.
- The CLI's `mt move` does for one slot what the keyboard move does. Planning it through
  `MoveSlots` would make its relocation keep the selection although the moved slot need not be the
  focused one, so it would first need its own way to collapse the selection.

## Considered options

- **A modal move mode first.** A key such as Cmd+M would enter the mode, plain arrows would move,
  Enter would confirm, and Esc would restore. Deferred, because a mode needs its own state, an
  indicator, and an Esc that takes precedence over escaping (ADR 0031), while the chord delivers
  the operation without them. A later decision can add the mode on top of `MoveSlots`.
- **Move into free space only.** The key would do nothing while any slot in front of the block is
  assigned. Rejected, because in a densely filled project almost every move would be blocked, and
  tiling window managers and text editors never block a move on content either.
- **Push the content in the way.** Nothing would ever block, but slots beyond the strip in front
  would change places as a side effect, and a move back would not restore them.
- **Leave or enter nested projects at their edges.** This would mirror how navigation escapes to the
  hosting project. Rejected for now, because it needs free space in another matrix and a new focus
  path, while a move within one matrix stays one simple operation.
- **Opt+Cmd+arrow or Ctrl+Cmd+arrow.** Opt+Cmd+arrow is unused, but the user preferred a chord with
  Ctrl. Ctrl+Cmd+↑ and Ctrl+Cmd+↓ already zoom, so Ctrl+Cmd+arrow would have to move the zoom keys
  elsewhere.
