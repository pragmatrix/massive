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
  [ADR 0008](../adr/archive/0008-task-local-ui-contexts.md)).
- A view gets its `ViewId` when it is created. The id is a random UUID, unique across the desktop
  ([lib.rs:46](../../applications/src/lib.rs#L46)).
- The desktop lays out projects, launchers and instances through presenters that implement
  `TargetLayout` and read the desktop's state through a `LayoutContext`
  ([ADR 0022](../adr/archive/0022-presenters-own-the-layout-of-their-targets.md)).
- One tree, `OrderedHierarchy<DesktopTarget>`, serves the structure, the focus, the layout, hit
  testing and scene parenting ([desktop_system.rs:289](../../desktop/src/desktop_system.rs#L289)).

## Decisions

### Layout metadata

- **Metadata is a Rust API on the view.** A view declares its panel index, its stack index, its
  ports, and its outgoing connectors through calls on the view.
- **Metadata travels with the view changes.** It goes into the instance's submissions, in order with
  the scene changes, through the channel that exists today. Its types live in
  `massive-applications`. They are plain data, so they also cross the boundary to an instance that
  runs out of process.
- **The indices count within the instance.** Panel 0 of one instance and panel 0 of another are
  different panels.
- **The primary view defaults to panel 0, stack 0.** An instance with one view therefore declares
  nothing. Any other view stays hidden until its instance gives it indices. An instance that gives
  the indices when it creates the view never sees a hidden phase.
- **A port is a name and a rect.** It belongs to a view. Its name is unique within that view, and
  its rect is in the view's own coordinates. A rect of zero size is a point. The view updates the
  rect when its content moves.
- **A connector is declared on its source view.** Its source end names an optional port of that
  view. Its target end names a view of the same instance by its `ViewId`, and optionally one of that
  view's ports by name. Connectors into other instances are a follow-up.
- **A connector stays hidden until both ends resolve.** An end does not resolve while its view does
  not exist or has no placement, or while the named port is not declared yet.
- **Applications bring only layout metadata.** They never bring a stage or a layouter, and they
  never position a view. They also never receive the layout focus, which stays in the desktop.

### The stage tree

- **The stage tree holds the layout.** The target tree keeps the structure and the focus. The stage
  tree holds the stages, the panels, the title bars and the placed views, and hit testing walks it
  ([ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md)).
- **The stage tree is derived.** A `SyncChildren` effect gives each stage its direct children
  whenever they change in the target tree or in the layout metadata. For a visor stage, the children
  are the panels of its instances.

### The generic layout stage

- **One `LayoutStage<L: Layouter>` implements every layout stage.** It handles the protocol with
  both sides, skips runs whose inputs did not change, and sets its children's placements.
- **A stage learns everything from its two sides.** It keeps state of its own and never reads the
  desktop's aggregate, the target tree or a `LayoutContext`.
  - Through `SyncChildren`, it receives its direct children, only when they change.
  - From its parent stage, it receives its placement, the layout focus, the animation ticks, and
    values its parent settles for it. For a visor stage, these include the connector lanes that the
    connector stage needs ([07](07-connector-routing.md)).
  - To its parent side, it reports its extent and what its layouter reports. A visor stage reports
    its center visor view, its panel structure, and its remembered views.
- **Every node animates its own placement.** A generic placed node with a scene location, a movement
  and a fade serves stages, panels, title bars and views. While it moves, it reports its current
  value every tick, and the desktop core composes these values for the connector stage
  ([08](08-animation-and-drawing.md)).
- **Stages receive no keys.** The keys that a launcher or a project handles are commands of their
  targets and stay in the desktop core.

### Traits

- **A `Stage` trait** is the interface through which the desktop core holds the stage tree,
  whatever the kinds of its stages. `LayoutStage<L>` and the connector stage implement it.
- **A `Layouter` trait** is the extension point for new arrangements. A layouter is a pure function.
  It takes the stage's state, inputs and events, and it returns the new state and the output. The
  output consists of the children's placements with their visibility, plain decoration that cannot
  be hit, such as a background, and what the stage reports to its parent side.
- **A layouter is not incremental.** It recomputes its stage in full, and the stage runs it only
  when an input changed.
- **Both traits live in `massive-desktop`**, because only the desktop implements and calls them.

## Testing

- Each layouter is tested as a pure function with plain values, including its state transitions.
- The generic layout stage is tested by feeding both sides and checking what it reports and which
  runs it skips.
- `SyncChildren` is tested against the rules where the stage tree differs from the target tree.

## Open questions

- The message types of the `Stage` trait.
- Whether the metadata calls live on the view or on `InstanceContext`.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- Whether a stage will need a say about key presses.
- Whether mt can register a layouter.
