# Exceptional desktop behaviors are decided by the desktop policy

## Status: accepted

Builds on [ADR 0012](./0012-a-configuration-without-a-launcher-is-given-one-at-load.md) and
[ADR 0014](./0014-fullscreen-mode-is-per-launcher-content-scaling.md). Both stay in force. This ADR
only decides where their rules are made.

## Problem

Most desktop commands do what the user asked for. A few do something else. Cmd+Shift+W keeps a
launcher when instances run below it, Cmd+Enter zooms before it acts, and a configuration without a
launcher is given one at load. These exceptional behaviors are the decisions users notice and question most,
and the ones most likely to change once the desktop can tell the user why something happened
(`docs/desktop-notifications-plan.md`).

Today each exception sits where it happened to be needed. Some are in command planning, some in a
presenter's key handler, some in the run loop, and some in the configuration load. Nothing shows
which branches are deliberate choices and which are mechanics. A refusal returns no change and no
reason, so a later notification has nothing to show.

## Decision

**A desktop policy module, `when` (`desktop/src/policy/when.rs`), decides every exceptional behavior.**
Each rule is a pure function named after the situation it decides, so a call reads as
`when::ungrouping(…)` or `when::booting(…)`. It takes the facts it needs as parameters
and returns a decision. The code that used to decide the rule now gathers those facts, asks the policy, and
carries out the answer.

**A behavior is exceptional when it could reasonably have been decided differently.** An operation
that the model makes impossible is an invariant, not a policy. It stays in the model. For example,
the root project sits in no slot, so it can be neither removed nor ungrouped. That is not a rule the
policy decides.

**A rule decides what the caller should not, and nothing more.** It takes the few facts behind its
decision, mostly flags, and returns an action enum named after its situation. The caller derives
the facts, and it finds or builds what the chosen action needs. A rule never selects a value the
caller hands it. The `when` module therefore imports almost nothing from the desktop.

```rust
match when::pressing_command_enter(fully_zoomed_in) {
    PressingCommandEnter::Act => …,
    PressingCommandEnter::ZoomFirst => …,
}
```

- The enum holds only the outcomes the rule can produce, so a caller handles every one of them.
- A refusing variant names its reason, so a notification can later tell the user why nothing
  happened.
- No rule returns `Result`, `Option`, or a shared verdict type. A refusal is an expected answer,
  not an error, and `Some` or `None` does not say what happens.
- A rule that ranks candidates returns them in order of preference, such as
  `[Neighbor(Right), …, HostingProject]`. The caller takes the first that exists, so each search
  keeps its home in the code that owns the data.

### The rules

Guards refuse a command when they apply.

- **Removing slot content while instances run below it is refused** (Cmd+Shift+W). A key press never
  quits running processes.
- **Ungrouping acts on the focused nested project, or on the project whose slots are all
  selected, and is refused otherwise.** The user then sees which slots move (ADR 0026). An
  extended selection is deliberate, so a project it covers entirely wins
  over a focused one. A project whose only slot holds the focus counts as fully selected.

Redirects run another command when they apply.

- **Ungrouping an empty project clears its slot instead.** The project has no slots to move, so only
  its removal is left.
- **Cmd+Enter zooms to `Focus` first and acts only when the target is fully in.** The presenters that
  handle Cmd+Enter pass in whether the camera is fully in and stop deciding the rule themselves.

The routing rule picks where an action goes.

- **Ctrl+Cmd+F routes to the native window or to Full Screen Mode.** A windowed window enters native
  fullscreen. A fullscreen window toggles Full Screen Mode when keyboard focus is fully in on an
  instance, and otherwise leaves native fullscreen (ADR 0014).

Lifecycle defaults return their own values.

- **A configuration without a launcher is given one at load.** The load step detects the
  situation and builds the default launcher (ADR 0012).
- **The startup launcher falls back to the first launcher depth-first** when its address path no
  longer resolves.
- **A stopping instance that holds the focus hands it to its neighbor toward the launcher's
  beginning,** or to the launcher when it has no neighbor. The hierarchy still finds the neighbor.
- **Clearing a slot that holds the focus moves the focus to the nearest assigned slot** in the
  order right, left, down, up, or to the hosting project when there is none. Matrix navigation
  still searches the slots.

## Consequences

- Every exceptional behavior can be read in one file and tested without a desktop system.
- A refusal now has a reason. Showing it is left to the notifications plan.
- Changing an exception means changing one rule. The code that carries the rule out may still need
  to support the new answer.
- The two focus replacement rules search in opposite directions. A stopping instance looks toward
  the beginning, a cleared slot looks right first. This ADR records both as they are. Placing them
  side by side makes the difference visible, but unifying them is a separate decision.
- A new exceptional behavior goes into the policy, and a new invariant goes into the model. The
  test above decides which is which.

## Considered options

- **A filter in front of command planning.** Every command would pass through the policy before it
  is planned, which gives one entry point. Rejected, because the Cmd+Enter redirect is decided in a
  key handler before any command exists, and lifecycle defaults are not commands.
- **One verdict type for every rule about a command,** with the outcomes allow, refuse, and
  redirect. Rejected, because no rule produces all three. Every caller would either handle outcomes
  its rule never gives or silently ignore them, and the type could not show which outcomes a rule
  really has.
- **A policy value with settings.** This would prepare for user-configurable rules. Rejected for
  now, because no rule is configurable yet and the settings would be speculative.
- **Guards and redirects only.** This keeps the module about commands. Rejected, because the
  lifecycle defaults are the same kind of choice. Leaving them out would keep the choices users
  notice most, such as which target takes over the focus, scattered across the code.
- **Include the invariants as guards.** Refusing to remove the root project would then also carry a
  reason. Rejected, because the policy would mix choices with structural facts, and the test for
  what belongs in it would be lost.
