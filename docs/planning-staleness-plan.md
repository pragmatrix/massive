# Planning staleness plan

Status: **draft, not approved**. Written from a session discussion on 2026-10-23. No code has
changed yet.

## Goal

A plan should never read state that lags behind the changes it follows. Today a plan can do so
without anyone noticing. ADR 0035 records this as a known limitation of the planner's design that
affects more than the shared column counts. This plan lists the ways the staleness arises, compares
the options to remove it, and recommends a combination.

## Terms

- A *transaction* is one call of `DesktopSystem::transact` in `desktop/src/desktop_system.rs`. It
  applies a sequence of desktop changes and then runs the effects they caused.
- A *desktop change* (`DesktopChange` in `desktop/src/desktop_system/change.rs`) is one update of
  the desktop system's state. A configuration change is one kind of desktop change.
- *Planning* turns a command into desktop changes and only reads state. `DesktopSystem::plan` in
  `desktop/src/desktop_system/command_dispatch.rs` plans every `DesktopCommand`. Configuration
  commands go on to `ConfigurationPlanning` in `configuration_planning.rs`.
- *Applying* a desktop change updates the state. `DesktopSystem::apply_change` does it. Applying can
  return follow-up changes. The transaction applies them next, before the changes that were already
  waiting.
- *Effects* are the layout steps a transaction runs after it applied all changes. They are
  `Measure`, `Place`, `Reparent`, and `ApplyLayout` in `effects.rs`. The camera is resolved after
  the effects.

## Current situation

### State becomes current at three moments

- *Source state* is current as soon as the change that sets it is applied. It covers the desktop
  configuration, the keyboard focus, the selection, the zoom level, and the window state.
- *Apply-derived state* is current as soon as the change it derives from is applied. It covers the
  hierarchy, the presenters, and the shared column counts. `apply_project_change` refreshes the
  counts in the same step as the configuration change.
- *Effect-derived state* is current only after the transaction ran its effects. It covers the layout
  placements in `DesktopLayoutState` and the camera.

`ConfigurationPlanning` already borrows only source and apply-derived state. Its documentation says
that the window, the layout, and the camera stay out of its reach. Most other plans borrow the whole
`DesktopSystem` and can reach everything.

### The staleness takes three shapes

1. **Several commands are planned against the same state.** The boot replay in
   `desktop/src/desktop.rs`, consumed keys in `key_delivery.rs`, and `ForwardEvents` in
   `apply_change` plan their commands one after another and concatenate the changes. A command
   planned second does not see the changes of the first.
2. **Plans run in the middle of a transaction.** `apply_change` plans commands while it applies
   `ForwardEvents` and instance submissions. The earlier changes of the transaction are applied at
   that point, but its effects have not run. Effect-derived state is therefore stale.
3. **The changes of one plan are computed from the state before the first of them applies.** The
   planner has to simulate the earlier changes by hand. The documentation of
   `ConfigurationChange::Ungroup` shows this. It relies on `MoveSlot`s planned before it to push
   aside the slots in the way. Where simulating became too hard, a compound change was added
   instead, such as `Group`, `SwapSlots`, and `RelocateSlots`.

### What holds today

A search of the planners finds no read of effect-derived state. `plan_zoom` and `plan_escape` use
only the topology queries of `ZoomGeometry` in `navigation/zoom_geometry.rs`. Placements are read to
resolve the camera after the effects, and once during applying.

The read during applying is step 2 of a re-parent, `Reparenting::apply` in `reparenting.rs`. It
captures the previous parent's placement before the effects run. It wants the placement from before
the transaction, because the content is still shown there.

The convention is unprotected, though. `ZoomGeometry` offers topology queries and placement queries
side by side, so a plan can start reading stale placements without anyone noticing.

## Options

### A. Plan commands when the transaction reaches them

The transaction holds commands as well as changes, for example through a variant
`DesktopChange::Plan(DesktopCommand)`. A command is planned when the transaction reaches it, so it
sees every change before it applied. This generalizes what `ForwardEvents` already does. It fixes
shape 1 at little cost. A plan that fails then fails in the middle of the transaction. That is
already possible today, because `transact` is not atomic.

### B. Restrict every plan to source and apply-derived state

Every plan gets a read-only planning view in the manner of `ConfigurationPlanning`, without
`DesktopLayoutState` and the camera. `ZoomGeometry` is split into a topology part for planning and a
placement part for camera resolution. Whatever needs layout becomes an intent that is resolved after
the effects, as `transact` already does for the camera.

This fixes shape 2 for good, because the type system rules stale reads out. It also settles the
first open question of ADR 0035 in favor of refreshing the counts during apply, which the code does
today. A refresh run as an effect would turn the counts into effect-derived state.

### C. Run pending effects before a plan reads layout

A plan that needs layout first runs the pending effects, the way a browser runs layout when a script
reads a size. This is correct but can cost several layout passes per transaction. It also
complicates layout animation and camera moves. It changes what step 2 of a re-parent captures, too.

### D. One transaction per command

Each command is planned, applied, and laid out before the next one starts. This is the simplest way
to get fresh state. It changes behavior that relies on batching, however.

- A transaction keeps a selection that one of its own changes set (ADR 0025, ADR 0033).
- The configuration is persisted once per transaction.
- Intermediate layouts would start animations that the next command redirects.

### E. Plan against a working copy

The planner applies its own changes to a scratch copy of the source and apply-derived state while it
plans. This ends the hand simulation of shape 3. With cheap snapshots, it would also make `transact`
atomic, which its comment says it is not yet. The cost is that applying must become a pure state
update. Today it also updates presenters and talks to the instance manager.

### F. Let changes resolve their positions when applied

A change names stable ids and an intent, such as placing content C next to content D, instead of
fixed slot positions. Such a change does not go stale as easily. Logic moves from planning into
applying, however. The compound changes are already a step in this direction.

### G. Detect stale reads

A debug assertion fails when effect-derived state is read while it is stale. A flag marks that
window. It is set as soon as an applied change invalidates layout, and it is cleared when the
effects start, because the effects bring layout up to date. This fixes nothing, but it turns silent
wrong reads into failures. Step 2 of a re-parent reads placements on purpose and needs an exemption.

## Recommendation

Make B the rule, use A for commands planned one after another, and add G as a guard.

- B turns the convention that `ConfigurationPlanning` already follows into one that holds for every
  plan, and the type system enforces it. It also settles the first open question of ADR 0035.
- A is small and stays inside the transaction loop.
- G protects the convention cheaply until B is in place.

Keep E in reserve until atomic transactions or undo become a goal, because that is when its cost
pays off. Reject C and D, because they get fresh state by giving up batching, and the selection and
camera rules depend on batching. F stays a tool for single cases, as it is today.

## Steps

Each step is small and can be reverted on its own.

1. **Guard (G).** Add the debug assertion and its flag, with the exemption for step 2 of a
   re-parent. Run the system tests to see whether any read trips it.
2. **Planning view (B).** Give the plans outside configuration planning a planning view that borrows
   only source and apply-derived state. Split `ZoomGeometry` into topology queries and placement
   queries, and keep only the topology part in planning.
3. **Commands in the transaction (A).** Let the transaction hold commands and plan each one when it
   reaches it. Move the boot replay, consumed keys, and `ForwardEvents` onto this.
4. **Decision record.** Record the rule from B in an ADR, and resolve the first open question of
   ADR 0035 there.

## Open decisions

- **Whether G stays once B is in place.** With B, plans cannot reach placements anymore, so G would
  only guard reads during applying.
- **How a command that fails to plan affects its transaction.** Today a failure aborts the rest of
  the transaction and leaves the earlier changes applied. A turns more plans into steps of a running
  transaction, so this case becomes more common.
- **Whether a boot replay command relies on being planned against the state before the replay.**
  Step 3 has to check this before it moves the boot replay.
