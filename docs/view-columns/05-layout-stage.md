# 05 Layout stage

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [03](03-multi-view-instances.md).

## Goal

A layout stage on the instance's side owns the layout of an instance's views. The application only
declares its views and their metadata. The stage decides where the views go, tells the desktop, and
draws the connectors. Any application can use it, and another layouter can replace the column
layouter.

## Current situation

- An instance sends its changes through `InstanceContext::submit`
  ([instance_context.rs:267](../../applications/src/instance_context.rs#L267)). View changes travel
  as typed `InstanceChange`s in the task's change queue
  ([instance_environment.rs:95](../../applications/src/instance_environment.rs#L95),
  [ADR 0008](../adr/0008-task-local-ui-contexts.md)).
- An instance receives events through `InstanceContext::wait_for_events`
  ([instance_context.rs:201](../../applications/src/instance_context.rs#L201)). Animation ticks
  arrive there, and movements advance on them
  ([ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).
- A view learns about the keyboard focus through `ViewEvent::Focused`
  ([view_event.rs:26](../../applications/src/view_event.rs#L26)).
- Views hang from a parent location that `InstanceContext::view` uses internally
  ([instance_context.rs:231](../../applications/src/instance_context.rs#L231)). Connectors need to
  be drawn below the views under that location.

## Decisions

- **The stage sits between submissions and events.** It sees the application's view changes before
  they reach the desktop, and it sees focus events and animation ticks before the application does.
  It hooks in at `InstanceContext::submit` and `wait_for_events`.
- **The application opts in.** It creates its views through the column layout. An application that
  does not opt in behaves as today.
- **Metadata is a Rust API.** A view declares its outgoing connectors and its ports through calls on
  the view. The metadata stays inside the instance and never crosses into the desktop.
- **A connector names its ends.** Its source is the declaring view, optionally with one of its ports.
  Its target is another view of the same instance, optionally with one of that view's ports.
- **A port is a name and a rect.** The rect is in the view's own coordinates. A rect of zero size is
  a point. The view updates the rect when its content moves.
- **The layouter is a trait.** A pure `Layouter` trait in `massive-applications` turns the views'
  metadata and the keyboard focus into view positions, connector routes, the content extent, and
  focus companions. The column layouter ([06](06-column-layouter.md)) is its first implementation.
  A general stage abstraction waits until a second stage needs one.
- **The stage keeps the selection.** It tracks `ViewEvent::Focused` and keeps one selected view per
  column. The layouter receives the selection as input.
- **The presentation layer is part of the stage.** It sends the layouter's results and animates them
  ([08](08-presentation-layer.md)).
- **Nothing new crosses the boundary.** Only view positions, the content extent, focus companions,
  and scene changes go to the desktop. So the stage works for applications that run out of process.

## Testing

The stage transforms one stream of changes and one stream of events. Tests feed both streams and
check what reaches the desktop and what reaches the application.

## Open questions

- How the application reaches the stage. It could wrap `InstanceContext`, or the stage could be a
  value the application calls.
- Whether the stage hides layout-related events, such as `Focused`, from the application or passes
  them on.
- How the stage reaches the location that views hang from, which is internal today.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- The shape of the `Layouter` trait's input and output types.
