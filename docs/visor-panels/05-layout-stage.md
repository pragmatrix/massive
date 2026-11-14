# 05 Layout stage

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [03](03-panels-on-the-visor.md). Records the parts of
[ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md) that the visor panels need.

## Goal

An application only declares the layout metadata of its views. A layout stage in the desktop
decides where the views go. One generic layout stage serves every level of the desktop, and a pure
layouter supplies what differs between the levels. The visor stage is the first one that this plan
needs ([06](06-visor-layouter.md)).

## Current situation

- An instance sends its changes through `InstanceContext::submit`
  ([instance_context.rs:264](../../applications/src/instance_context.rs#L264)). View changes travel
  as typed `InstanceChange`s in the task's change queue, interleaved with scene changes
  ([instance_environment.rs](../../applications/src/instance_environment.rs),
  [ADR 0008](../adr/0008-task-local-ui-contexts.md)).
- An instance receives events through `InstanceContext::wait_for_events`
  ([instance_context.rs:198](../../applications/src/instance_context.rs#L198)).
- Views hang from a parent location that `InstanceContext::view` uses internally
  ([instance_context.rs:228](../../applications/src/instance_context.rs#L228)).
- The desktop lays out projects, launchers and instances through presenters that implement
  `TargetLayout` and read the desktop's state through a `LayoutContext`
  ([ADR 0022](../adr/0022-presenters-own-the-layout-of-their-targets.md)).

## Decisions

### Layout metadata

- **Metadata is a Rust API on the view.** A view declares its panel index, its stack index, its
  ports, and its outgoing connectors through calls on the view.
- **Metadata travels with the view changes.** It goes into the instance's submissions, in order with
  the scene changes, through the channel that exists today. It is plain data, so it also crosses the
  boundary to an instance that runs out of process.
- **The indices count within the instance.** Panel 0 of one instance and panel 0 of another are
  different panels.
- **The primary view defaults to panel 0, stack 0.** An instance with one view therefore declares
  nothing. Any other view stays hidden until its instance gives it indices. An instance that gives
  the indices when it creates the view never sees a hidden phase.
- **A connector names its ends.** Its source is the declaring view, optionally with one of its ports.
  Its target is a view, optionally with one of that view's ports. A target in another instance
  names that instance too. An instance knows the ids of the nested instances it started
  ([ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md)).
- **A port is a name and a rect.** The rect is in the view's own coordinates. A rect of zero size is
  a point. The view updates the rect when its content moves.
- **The application never positions a view.** It also never receives the layout focus, which stays
  in the desktop.

### The generic layout stage

- **One `LayoutStage<L: Layouter>` implements every layout stage.** It handles the protocol with both
  sides, skips runs whose inputs did not change, animates the placements and their visibility, and
  applies them to scene locations ([08](08-animation-and-drawing.md)).
- **A stage learns everything from its two sides.** It keeps state of its own and never reads the
  desktop's aggregate or a `LayoutContext`.
  - From the desktop core, it receives its own direct children, only when they change.
  - From its parent stage, it receives its placement, the layout focus, the animation ticks, and
    values its parent settles for it. For a visor stage, these include the lane counts of the
    connector stage ([07](07-connector-routing.md)).
  - To its parent side, it reports its extent and what its layouter reports. A visor stage reports
    its center visor view and its panel structure.
- **It reports its current placements while they move.** Every tick in which a placement animates,
  the stage reports the current values. The desktop core composes them once per tick, and connector
  stages read them ([07](07-connector-routing.md)).
- **Stages receive no keys.** The keys that a launcher or a project handles are commands of their
  targets and stay in the desktop core.

### Traits

- **A `Stage` trait** is the interface through which the desktop core holds the tree of stages,
  whatever their kind. `LayoutStage<L>` and the connector stage implement it.
- **A `Layouter` trait** is the extension point for new arrangements. A layouter is a pure function.
  It takes the stage's state, inputs and events, and it returns the new state and the output. The
  output consists of the children's placements with their visibility, plain decoration such as a
  title bar or a background, and what the stage reports to its parent side.
- **A layouter is not incremental.** It recomputes its stage in full, and the stage runs it only
  when an input changed.
- **Both traits live in `massive-applications`.**
- **An application may bring a stage of its own.** It runs only in the application's task and
  computes the layout metadata of the application's views.

## Testing

- Each layouter is tested as a pure function with plain values, including its state transitions.
- The generic layout stage is tested by feeding both sides and checking what it reports and which
  runs it skips.

## Open questions

- The message types of the `Stage` trait.
- Whether the metadata calls live on the view or on `InstanceContext`.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- Whether a stage will need a say about key presses.
- Whether mt can register a stage that runs in the desktop's task.
