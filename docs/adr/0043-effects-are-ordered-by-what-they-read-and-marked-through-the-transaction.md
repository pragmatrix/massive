# Effects are ordered by what they read and marked through the transaction

## Status: accepted

## Context

A desktop transaction first applies its changes and then runs everything those changes make
necessary: measuring and placing targets, refreshing the shared column counts and the MCP
indicators, collapsing the selection, moving the camera, updating the hover outline, and asking the
host to persist the configuration. Three mechanisms used to carry that work.

- **`ChangeSurface`** collected what the changes touched. It had nine fields: sets of targets and
  projects, flags, and an ordered impact. `transact` split it into effects, system effects, and a
  few flags of its own.
- **The effect scheduler** ran `DesktopEffect`s in four hand-ordered phases. A handler returned its
  follow-up effects, and an effect scheduled into a phase that had already run panicked.
- **The tail of `transact`** ran the remaining steps in an order written by hand: collapsing the
  selection after comparing the zoom level and focus before and after, setting the desired camera,
  synchronizing the camera, laying out and showing the zoom level indicator, and updating the hover
  placement.

The order of these steps was spread over the phase enum, the field list of `ChangeSurface`, and the
sequence of statements in `transact`, and none of them stated what a step depends on.

The renderer also computes dependent values, its location matrices and view projections. It
computes them on demand every frame, keyed by version numbers. That is a different evaluation
strategy on a different path, and it is not part of this decision.

## Decision

The desktop runs all work that follows from a transaction's changes as **effects**, scheduled by
one **effect scheduler**.

- An effect is a value without data that names work still to do: its kind and, for most kinds, a
  subject, such as `Measure(target)`.
- Code **marks** an effect through a task-local service that `transact` installs for one
  transaction. Marking only appends the effect to the **pending effects**.
- Each effect kind declares the effect kinds whose results it reads. The topological order of
  these reads gives each kind its **rank**.
- The scheduler repeatedly takes the pending effects of the lowest rank as one **rank batch**,
  removes duplicates, and handles them, until no effects are pending.

A transaction has two stages. All changes run first, including the follow-up changes they produce.
Then `transact` drops the pending effects of targets that a later change removed, and runs the
scheduler. Changes are not effects, because a change carries data and is not idempotent: two
"move the selected slots left" changes are not the same as one.

`ChangeSurface` and the hand-written tail of `transact` are gone. Facts that a change records for
the effects of the same transaction, such as "this transaction set the selection", live in the
desktop state and are reset when the transaction ends.

## Features of the effect scheduler

Each feature is listed with the case that requires it. A feature without such a case is not part
of the scheduler.

### 1. An effect is a value without data

An effect consists of its kind and, for most kinds, a subject. It carries no other data.

Why: two pending effects that are equal can only be merged when neither carries data that the other
lacks. Merging them is what keeps a transaction from measuring the same matrix once per change.
Data that the handling of an effect needs from the change that caused it lives in the desktop state
under the same identity. `Reparent(target)` works this way: the change stores the target's old
on-screen position in `Reparenting`, and the handling takes it from there.

### 2. Handling an effect recomputes from the current state

The handling of an effect reads the current desktop state and brings the state that depends on it
up to date. Handling the same effect twice in a row has the same result as handling it once.

Why: the scheduler merges duplicates and chooses the order itself. Both are safe only when the
outcome does not depend on how often or in which order an effect was marked. This is also why
changes are not effects.

### 3. Marking through a task-local service

Code marks an effect by calling `mark` on it. `mark` reaches the pending effects through a
task-local, in the way ADR 0008 installs the scene's change queue. `transact` installs the scope
for the duration of one transaction. A mark outside that scope panics.

Why: the places that change what effects depend on sit deep inside the desktop system, for example
setting the focus or the zoom level. Passing a collector down to each of them would thread a
parameter through code that does not otherwise need it. Scoping the service to one transaction
means that the pending effects start empty every time, and that a mark made outside a transaction is
reported at the place that made it instead of waiting silently for the next transaction. ADR 0008's
benchmark measured about 2 ns per access to a task-local `RefCell`.

### 4. Marking only appends

The pending effects are kept in one `Vec` per rank. A mark looks up the rank of its kind in a table
and appends the effect to that rank's `Vec`. Marking never sorts, hashes, or searches.

Why: a transaction marks far more often than it handles, because many changes mark the same
effects. Handling an effect such as `Measure` costs far more than a mark. The cheap operation is
therefore kept as cheap as possible, and all work that can wait is done once, when the effects are
taken.

### 5. Duplicates are removed when a rank batch is taken

When the scheduler takes the pending effects of one rank, it keeps the first mark of each effect
and drops the later ones. A set of the effects seen so far tells them apart, so an effect only has
to be comparable for equality and hashable.

Why: one transaction often marks the same effect many times, for example `Measure` of a matrix for
every slot that a configuration change edits. Removing duplicates turns these into one handling.
Sorting would remove them as well, but it would require an order on every effect. The subjects of
the desktop's effects are random ids, which already are hashable but have no meaningful order.

### 6. Each effect kind declares what it reads

Each effect kind declares the effect kinds whose results its handling reads. The declaration is the
only source of the order in which kinds are handled.

Why: many effects read what others produced. `Place` reads the sizes that `Measure` produced, and
`ApplyLayout` reads the placements that `Place` produced. `Reparent` needs the final placement of
the target's new parent, even though no `Place` marks it. An order derived only from which kinds
mark which would miss that case. A kind marks another kind only because the other kind reads its
results, so a separate list of marks would add no information and could drift from the reads.

### 7. Ranks come from the topological order of the reads

The scheduler sorts the effect kinds topologically by their reads, once, in the first transaction,
which is the desktop's setup at startup. A kind's position in that order is its rank. A cycle
between different kinds panics there. A kind may read itself.

Why: a hand-maintained enum of phases fixed the order separately from what the effects depend on,
and the two could only be found to disagree at runtime. Deriving the order from the reads keeps one
source of truth and reports a contradiction when the desktop starts. A kind has to be allowed to
read itself, because `Measure` reads the sizes of its children and `Place` reads the placement of
its parent.

### 8. Rank batches

The scheduler repeatedly takes the lowest rank that has pending effects, handles all of them as one
rank batch, and starts again. It stops when no effects are pending.

Why: an effect that its own kind marks has to run after the effect that marked it. A child's
`Measure` marks its parent's `Measure` when the child's size changed, so `Measure` climbs one level
per rank batch, bottom-up. A parent's `Place` marks its child's `Place`, so `Place` descends
top-down. Effects of a lower rank are never marked while a higher rank is handled (feature 10), so
a rank batch never has to go back.

### 9. No required order within a rank batch

Effects within a rank batch are handled in the order of their first marks. Correctness does not
depend on that order.

Why: a rank batch holds effects of one kind, and handling an effect recomputes from the current
state. When handling one effect makes another effect of the same kind stale, it marks that effect
again, and the next rank batch handles it. The order only affects how much work is done: a parent
measured before its child in the same rank batch is measured again after the child. The marking
order is deterministic, so the same transaction always handles its effects in the same order.

### 10. A check of every mark

In debug builds, marking an effect of kind `K` while handling an effect of kind `A` panics unless
`K` is `A` or `K` declares that it reads `A`.

Why: marks are made through an ambient service, so nothing in the types ties them to the declared
reads. Without the check, a handler could mark a kind that is ordered before it, and that effect
would run against results that are not final yet, without any error.

### 11. Automatic marks for kinds without a subject

After handling an effect of kind `A`, the scheduler marks every effect kind that has no subject and
declares that it reads `A`.

Why: `SynchronizeCamera` and `UpdateHover` read many kinds, among them the placements, the
selection, and the desired camera. If every handler of those kinds had to mark them, a forgotten
mark would leave a stale hover outline or camera without any error. Marking them only when
something they read was handled means that a transaction that touches none of their inputs does not
run them at all. Kinds with a subject cannot be marked this way, because the scheduler cannot know
which subject is affected.

Automatic marks cover only inputs that effects produce. An input that no effect produces is marked
where it changes. The presented camera is such an input: a gesture freezes it behind its desired
view, so after the gesture `transact` marks `SynchronizeCamera` because the camera reports that it
lags behind, although no effect changed the desired view.

### 12. The borrow is released before an effect is handled

The scheduler takes a rank batch out of the pending effects and releases its borrow of the
task-local before it handles the batch.

Why: the handling marks further effects through the same task-local. ADR 0008 makes re-entrant
access to a task-local fail loudly, so a borrow held during handling would panic on the first mark.

### 13. A read-only transaction context

Handling receives a read-only context that holds the transaction's effects mode, whether layout
animates, and the camera presentation mode.

Why: `ApplyLayout` animates only when the transaction permits it, `SynchronizeCamera` needs the
camera presentation mode, and persisting the configuration is skipped in setup transactions.

### 14. An outbox for system effects

Handling an effect may append a **system effect** to an outbox, which `transact` returns to the
host. System effects are what the desktop asks its host to do: persist the configuration, set the
window's fullscreen state, and announce changed MCP roots.

Why: these are consequences of the changes like any other effect, so they are marked and
deduplicated the same way. The scheduler itself knows nothing about host capabilities, which keeps
it testable on its own. Fullscreen is requested as a desired state instead of a toggle, so that
handling it twice has the same result as handling it once.

### 15. The first error aborts the transaction

When handling an effect returns an error, the scheduler stops and returns it.

Why: this was the behavior of the previous effect loop, and nothing in this decision changes it.

### 16. Generic over the effect type, located in the desktop

The scheduler's core is generic over the effect type and lives in a module of the desktop crate.

Why: the core can be tested on its own with small effect types, and the desktop is its only user. It
moves into `massive-util` when a second user appears.

## Considered options

- **One engine for the renderer and the desktop.** The renderer computes on demand every frame,
  and the desktop runs eagerly once per transaction. One engine for both would need on-demand
  recomputation as well as pushed invalidation, which makes its core more complex and adds a
  dependency to the renderer's per-frame path.
- **Dependencies tracked automatically through accessors.** Every read of desktop state would go
  through a wrapper that records it. That is intrusive and adds a cost to every read, while the
  declared reads are few and checked (feature 10).
- **Changes as effects.** Changes carry data and are not idempotent, so the scheduler would need a
  second kind of queue that keeps duplicates and their order. Running all changes first also lets
  `transact` drop effects whose target a later change removed.
- **Ordering by declared marks, or by a hand-maintained phase enum.** Both miss reads that no mark
  expresses, as with `Reparent` (feature 6).
- **Asking whether an effect is pending.** Handling would then depend on whether another effect was
  already handled, which ties the outcome to the order. Facts such as "this transaction set the
  selection" live in the desktop state instead.
- **Comparing the zoom level and focus before and after the transaction.** That reacts only to the
  net change, but it is a second mechanism next to marking. The places that set the focus and the
  zoom level mark instead.
- **A hash set as the store of pending effects.** Marking would hash every effect, and the set's
  iteration order changes between runs. A hash set only serves to recognize duplicates when a rank
  batch is taken (feature 5).
- **Versions.** All work happens inside one transaction, so knowing what is pending is enough.

## Consequences

- A focus that moves away and back within one transaction now counts as a change. It collapses the
  selection and shows the zoom level indicator, where comparing before and after did not.
- `Measure` and `Place` are separate ranks, so every size is final before the first placement.
  A parent and its child that are pending in the same rank batch may be measured or placed twice,
  depending on which was marked first. The result is still correct (feature 9).
- The zoom level indicator is laid out at the window size after the transaction's changes, because
  its effect reads the current state.
- The desired camera is resolved again after every rank batch of `Measure` or `Place`, including
  measures that only other effects marked, such as a refreshed column count.
- The facts a change records for the effects of its transaction (the selection was set, the window
  was resized, the requested fullscreen state) live together in one record of the desktop system,
  not in `Selection`. `Selection` is compared with its default to tell whether it is extended, so a
  flag inside it would break that comparison.
