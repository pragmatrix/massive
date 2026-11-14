# 05 Layout stage

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Depends on [03](03-multi-panel-instances.md).

## Goal

A layout stage between a multi-panel application and the desktop owns the layout of the
application's views. The application only declares its views, the panel and stack index of each,
and their connectors and ports. The stage decides where the views go, makes the panels slide, tells
the desktop, and draws the connectors. The application knows nothing about the stage besides which
stage it uses, and another layouter can replace the panel layouter.

## Current situation

- An instance sends its changes through `InstanceContext::submit`
  ([instance_context.rs:264](../../applications/src/instance_context.rs#L264)). View changes travel
  as typed `InstanceChange`s in the task's change queue
  ([instance_environment.rs](../../applications/src/instance_environment.rs),
  [ADR 0008](../adr/0008-task-local-ui-contexts.md)).
- An instance receives events through `InstanceContext::wait_for_events`
  ([instance_context.rs:198](../../applications/src/instance_context.rs#L198)). Animation ticks
  arrive there, and movements advance on them
  ([ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).
- A view learns about the keyboard focus through `ViewEvent::Focused`
  ([view_event.rs:26](../../applications/src/view_event.rs#L26)).
- Views hang from a parent location that `InstanceContext::view` uses internally
  ([instance_context.rs:228](../../applications/src/instance_context.rs#L228)).

## Decisions

- **The stage sits between submissions and events.** It sees the application's view changes before
  they reach the desktop, and it sees events before the application does. It hooks in at
  `InstanceContext::submit` and `wait_for_events`.
- **A multi-panel application uses the stage.** It creates its views through the stage. A
  multi-instance application does not use it and behaves as today.
- **Metadata is a Rust API.** A view declares its panel index, its stack index, its outgoing
  connectors, and its ports through calls on the view. The metadata stays inside the instance and
  never crosses into the desktop.
- **A connector names its ends.** Its source is the declaring view, optionally with one of its ports.
  Its target is another view of the same instance, optionally with one of that view's ports. The two
  views can be in different panels.
- **A port is a name and a rect.** The rect is in the view's own coordinates. A rect of zero size is
  a point. The view updates the rect when its content moves.
- **The layouter is a trait.** A pure `Layouter` trait in `massive-applications` turns the views'
  sizes and indices, the connectors, the remembered views, and the layout focus into panel sizes and
  offsets, view positions, and connector routes. The panel layouter ([06](06-panel-layouter.md)) is
  its first implementation. A general stage abstraction waits until a second stage needs one.
- **The stage keeps the remembered views.** It keeps one remembered view per panel, by the rules in
  [06](06-panel-layouter.md). The layouter receives them as input.
- **The stage follows the layout focus, not the keyboard focus.** The desktop sends the stage the
  view that holds the layout focus ([03](03-multi-panel-instances.md)). The layout focus waits while
  a mouse button is pressed, so a click never slides a panel under the pointer
  ([ADR 0002](../adr/0002-keyboard-focus-changes-are-command-mediated.md)). The stage's sliding
  also starts together with the desktop's centering of the arc. The application never sees this
  event.
- **The presentation layer is part of the stage.** It sends the layouter's results and animates them
  ([08](08-presentation-layer.md)).
- **Little crosses the boundary.** Towards the desktop go only panels, view positions, and scene
  changes. Towards the stage comes only the layout focus besides the existing events. So the stage
  works for applications that run out of process.

## Testing

The stage transforms one stream of changes and one stream of events. Tests feed both streams and
check what reaches the desktop and what reaches the application.

## Open questions

- How the application reaches the stage. It could wrap `InstanceContext`, or the stage could be a
  value the application calls.
- Whether the stage passes `ViewEvent::Focused` on to the application.
- How the stage draws a panel's part of the connectors relative to the panel, now that the desktop
  gives each panel its own scene location.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- The shape of the `Layouter` trait's input and output types.
