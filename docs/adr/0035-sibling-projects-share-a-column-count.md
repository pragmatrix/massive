# Sibling projects share a column count

## Status: proposed

Builds on [ADR 0023](./0023-matrix-slots-share-one-slot-extent-and-center-their-content.md), whose
slot extent and shared project scale stay in force, and on
[ADR 0034](./0034-a-project-header-does-not-widen-its-project.md), whose header measures no width.
Amends ADR 0023 in that a nested project no longer stretches horizontally to its slot, and ADR 0034
in that the horizontal alignment of a project's columns is no longer a policy. Amends
[ADR 0033](./0033-ctrl-shift-cmd-arrows-move-the-selected-slots.md): a selected block moves within
the project's shared column count instead of within its assigned slots.

## Problem

A nested project stretches to its slot (ADR 0023). In layout space, its width becomes the slot
extent's width divided by the shared project scale. When the widest sibling project sets that
scale, every sibling project ends up exactly as wide as that one, so they all show the same number
of columns. When a tall sibling sets the scale instead, the division yields a width that is no
whole number of columns. Every project then shows its columns plus a fraction of empty space, the
widest one included. The equal column count is therefore a side effect of the scale, not a rule.

A selected block also cannot move into the empty space a stretched project shows. ADR 0033 keeps
the block inside the rectangle of the project's assigned slots, so the columns that a wider sibling
gives the project are unreachable.

## Decision

**A project lays out as many columns as its widest sibling project.** A project's own column count
runs up to its highest assigned column, and an empty project holds one column. Its shared column
count is the largest own column count among the projects in its parent's matrix. The scope is the
whole parent matrix, not the parent's column, because the shared project scale has the same scope.
Launchers do not take part, because a launcher always fits one slot. The root project has no
siblings, so its shared column count is its own column count.

**Only columns are counted.** Rows keep their stretch in pixels and stay centered. Rows are as tall
as their tallest slot (ADR 0023), so a row count would not give sibling projects equal heights, and
ADR 0023 already rejected fixing row heights.

**Column counts are the source, and pixels only follow from them.** The configuration layer counts
columns. The layout layer turns a number of columns into a width in one place. No code derives a
column count from a width.

**A project's measured width already holds its shared column count.** A project's matrix measures
as many columns as its shared column count, so every sibling project measures the same width on
its own. A nested project therefore stretches only vertically. The measurement takes the counts
from the desktop configuration, not from allocated extents, so ADR 0023's rule that placement does
not feed back into measurement holds. A project's intrinsic scene now includes the columns its
siblings give it.

**The horizontal alignment policy goes.** A project is now exactly as wide as its columns and its
padding, so its matrix never has width beyond its columns to align them in. ADR 0034's policy for
that alignment has nothing left to decide.

**When a tall sibling sets the scale, a project presents narrower than its slot.** It shows exactly
its shared column count and sits centered in its slot, as every slot's content does (ADR 0023). The
empty space lies beside the project, not inside it.

**Moves and selections stay within the shared column count.** The rectangle a selection extends in
and a block moves in is as wide as the shared column count and as tall as the project's own rows. A
move still never raises the shared column count, but it can lower it: when the widest project's
rightmost content moves left, every sibling project narrows with it. Select All (Cmd+A, ADR 0030)
still selects the assigned slots only, because "all" means all content, not all space.

**Navigation does not change.** The keyboard focus moves between slot contents, and an unassigned
slot holds nothing to focus. A selection can therefore reach an empty column, but the focus cannot.

**A separate engine keeps the counts current.** It holds each project's own column count and each
project's shared column count, so reading a count takes constant time, for layout and planning
alike. The engine can also resolve everything from scratch, which boot uses. It is derived state
kept next to the desktop configuration, not inside it, so the configuration stays plain data. The
desktop system owns both.

**A configuration change names the projects it touches.** Those are the projects whose slots it
changes. An exhaustive match over the configuration changes lists them, so a new kind of change has
to name its touched projects too. A project that a change removes is marked as well, by the
application of the change, which alone knows the subtree a removal takes along. Its refresh finds
it no longer configured and drops its counts, so the counts only change in the refresh.

**The counts are refreshed in an effect phase before layout.** Each touched project becomes one
refresh effect, and refreshes of the same project within a transaction merge into one. The phase
runs after all changes of the transaction, so the configuration and the topology are final. A
refresh therefore reads the project's children from the configuration and its parent from the
topology instead of keeping its own copy of the tree, and the refreshes can run in any order. A
refresh recounts the project's own columns. It then shares the largest own column count among the
project's siblings and among the project's child projects, because those are the two groups whose
shared column count it can change.

**A refresh only re-measures, and it ends after two groups of matrices.** A project's own column
count depends only on its own slots, and a shared column count only on the own column counts of a
parent's child projects. When a refresh changes a shared column count, that project's matrix is
measured again. No other count changes, so a refresh never causes another refresh. The counts never
depend on layout, so nothing in layout causes a refresh either. The layout phase follows the
refresh phase, so every count is final before a matrix measures.

## Consequences

- Only matrices whose shared column count changed are measured again. A count that two refreshes
  of one transaction change and change back costs one measure that changes nothing.
- A transaction refreshes each touched project once. Boot replays the whole configuration in one
  transaction, so it refreshes every project once, linear in the number of slots.
- The engine is state that must not drift from the configuration. A test applies random
  transactions of configuration changes, refreshes them in a random order, and compares the engine
  with one resolved from scratch.
- A selection outline in an empty column needs no special case, because the matrix lays out every
  column of its shared column count as an interval.
- The planner plans commands in the middle of a transaction, after earlier changes of the same
  transaction were applied and before the transaction's effects ran. It can therefore read state
  that is only partly updated. This is a known limitation of the planner's design that affects more
  than the counts. The counts are refreshed by an effect, so a command planned after a
  configuration change in the same transaction reads counts from before that change. No such
  command reads them today. A block move and a selection extension are each planned from one key
  press before its own transaction, and boot plans all its commands before its transaction.

## Considered options

- **Keep the stretch to the slot extent divided by the scale.** Rejected, because a tall sibling
  then adds a fraction of a column to every project.
- **Stretch each project to its widest sibling's measured width.** Rejected, because it derives a
  column count from a pixel width, against the direction of the layers.
- **Stretch each project by the width of the columns it lacks.** Rejected, because the result still
  relies on reading the measured width as the project's own columns, and the measured width has to
  be right without a stretch.
- **Compute the shared column count on demand during measurement.** Each sibling would count all
  its siblings, and every change would have to re-measure the siblings whether their count changed
  or not. Rejected in favor of the engine, whose reads are constant and whose invalidation is
  precise.
- **Resolve all shared column counts into a table for each layout pass.** Rejected, because the
  layout builds its context for every target it measures or places. A pass over the configuration
  there would make a full layout quadratic in the number of slots.
- **Refresh the counts from the matrices layout already re-measures.** The touched projects are
  exactly those matrices, but configuration state would then follow layout invalidation, against
  the direction of the layers.
- **Refresh the counts together with each configuration change.** The counts would never be behind
  the configuration that planning reads. Rejected, because the topology lags the configuration
  while a transaction's changes run. The engine would then have to keep its own copy of the tree
  and keep that copy right in every refresh order. It would also refresh a project once per change
  instead of once per transaction.
- **Let the configuration own the counts and update them in its mutations.** No change could miss
  a refresh. Rejected, because the counts are derived state and the configuration stays plain data.
  The exhaustive list of touched projects closes the gap instead.
- **Keep a slot count per column for each project.** A single slot would then update in constant
  time. Not chosen, because recounting a touched project's slots is cheap and needs less state that
  must stay exactly right.
- **Count the columns per parent column.** Rejected, because columns of a parent could then differ
  in width while their projects share one scale.
- **Count rows as well.** Rejected, because rows differ in height (see above).
