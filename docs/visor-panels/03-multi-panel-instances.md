# 03 Multi-panel instances

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Records the desktop's side of
[ADR 0045](../adr/0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md).

## Goal

The desktop hosts multi-panel instances. It places their panels on the visor's arc, scales them,
and moves the keyboard focus between their views. It knows nothing about connectors, ports, the
remembered views, or how panels slide.

## Current situation

- Every launcher presents its instances as a visor
  ([ADR 0044](../adr/0044-launchers-present-only-as-visors.md)). The visor places them on an arc
  around its center visor instance and collapses to that instance when the focus leaves the launcher
  ([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs)).
- The arc's placement takes an instance's index and the number of instances
  ([visor_layout.rs:13](../../desktop/src/projects/visor_layout.rs#L13)). So it assumes that all
  instances are equally wide.
- An instance can present exactly one view. `present_view` rejects every view that is not the
  primary view, and a second primary view
  ([instance_presenter.rs:312](../../desktop/src/instance_presenter.rs#L312)).
- The desktop positions the one view itself. A view is a part of its instance
  (`InstancePart::View`), and the instance presenter lays it out
  ([ADR 0022](../adr/0022-presenters-own-the-layout-of-their-targets.md)).
- A view states its size when it is created (`ViewCreationInfo`). The desktop does not handle
  `ViewChange::Resize` yet
  ([command_dispatch.rs:845](../../desktop/src/desktop_system/command_dispatch.rs#L845)).
- The instance title bar shows the title of the primary view
  ([ADR 0019](../adr/0019-instance-title-bar-is-part-of-the-instance-extent.md)).
- Every launcher starts the desktop's primary application
  ([command_dispatch.rs:392](../../desktop/src/desktop_system/command_dispatch.rs#L392)).
- The desktop decides Full Screen Mode. It gives the view the window's size, and the instance only
  receives `ViewEvent::Resized`
  ([fullscreen.rs:41](../../desktop/src/desktop_system/fullscreen.rs#L41),
  [ADR 0014](../adr/0014-fullscreen-mode-is-per-launcher-content-scaling.md)).

## Decisions

### Applications and launchers

- **An application is multi-instance or multi-panel.** It states which as a fixed property of
  `Application`, next to its draw target
  ([application.rs:17](../../applications/src/application.rs#L17)). The registry knows it from
  startup, so the desktop never has to ask.
- **A multi-panel launcher runs one instance.** While its instance runs, the launcher refuses Cmd+T
  and Shift+Cmd+T.
- **A multi-instance instance presents one panel.** The desktop rejects a second panel from it, as
  `present_view` rejects a second view today.
- **The shell refuses multi-panel applications.** The shell shows one view per instance
  ([ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).
- **A launcher names its application.** An optional `application` field in the desktop
  configuration names the application the launcher starts. Without it, the launcher starts the
  primary application. The field changes the persisted configuration, the
  [KDL plan](../desktop-configuration-kdl-plan.md), and `desktop_describe`, and both security
  documents ([massive](../security.md), [mt](../../../docs/security.md)) are updated in the same
  change.

### What the stage sends

- **The one-view limit is lifted for multi-panel instances.** Such an instance can present any
  number of views.
- **The stage sends its panels in order.** Each panel carries its width, its height, and its
  vertical offset from the center line. It also names the view whose title its title bar shows,
  which is the panel's remembered view ([06](06-panel-layouter.md)).
- **The stage sends each view's panel and position.** The position is the view's top-left corner,
  relative to its panel. A layouter never resizes a view, so a view change carries only the panel
  and the position.
- **The desktop applies panels and positions as they arrive.** It does not ease them itself, because
  the stage animates the drawing of its views ([08](08-presentation-layer.md)).

### Panels on the arc

- **Each panel gets its own scene location.** The desktop moves and turns every panel on its own.
- **Panels sit edge to edge.** There is no gap between neighboring panels.
- **The arc places panels of any width and height.** The placement takes the panels' widths instead
  of an index and a count. Equal widths keep today's look.
- **The arc centers on the panel of the focused view.**
- **The center line lies at the launcher's vertical center.** Panels hang from it by their vertical
  offsets, and the arc turns panels only around the vertical axis, so the offsets stay valid.
- **Each panel has a title bar.** It shows the title of the view the stage names for the panel
  ([ADR 0019](../adr/0019-instance-title-bar-is-part-of-the-instance-extent.md) applies per panel).

### Scale

- **A multi-instance launcher scales as it does today.** Each instance is shown at its instance
  scale, the largest scale at which its primary view fits the slot. Expanding and collapsing never
  change that scale, and the camera cancels it at the Focus zoom level. Full Screen Mode stays as it
  is.
- **A collapsed multi-panel launcher shows only its primary view, fitted to the slot.** The primary
  view can have any size.
- **An expanded multi-panel launcher shows every panel at 1:1.** Panels may extend beyond the slot,
  also vertically.
- **The change between the two scales animates** with the structural animation.
- **The slot measures the fitted primary view.** So a launcher stays one slot extent
  ([ADR 0044](../adr/0044-launchers-present-only-as-visors.md)), and every row of the matrix keeps
  the slot extent ([ADR 0023](../adr/0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)).

### Focus and navigation

- **Hit testing uses the positions the stage sent.** A click hits a view at its position in its
  panel.
- **Cmd+arrows move to the neighboring view.** In an expanded visor, Cmd+←/→ cross panel edges, and
  Cmd+↑/↓ move within a panel. The neighbor is found by position.
- **Past the outermost view, navigation escapes to the matrix**, as it does today.
- **The desktop remembers one center visor view per launcher.** It generalizes the center visor
  instance. Re-entering the launcher focuses it, and the arc centers on its panel.
- **The desktop sends the layout focus to the stage.** When the layout focus moves to a view of a
  multi-panel instance, the desktop tells that instance's stage which view holds it. The stage
  updates its remembered views from it, and the application never sees the event
  ([05](05-layout-stage.md)).

## Glossary changes

- Redefine **panel**. Define **multi-instance application**, **multi-panel application**, and
  **center line**.
- Replace **center visor instance** by **center visor view**, and update **collapsed visor** and
  **non-center visor instance**.
- Update **instance title bar**, which today names the primary view.
- Add **instance scale** for how a multi-instance launcher scales.

## Open questions

- How a connector crosses from one panel to the next on the arc, and whether that changes the arc's
  shape, for example to flat panels hinged at their edges ([02](02-connector-shapes.md)).
- How Full Screen Mode works in a multi-panel launcher. Showing a window-sized primary view at 1:1
  would cover the neighboring slots while the visor is expanded.
- The names of the new view change and of the panel description.
- What happens to a view that is created but has no panel and position yet. It could stay hidden
  until they arrive.
- Whether `ViewRole` means more than "primary" for a multi-panel instance.
- Whether `desktop_describe` lists an instance's panels and views.
- How placement visibility ([ADR 0001](../adr/0001-placement-visibility-drives-interaction-and-fade.md))
  applies to several views and panels of one instance.
