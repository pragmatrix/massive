# 03 Panels on the visor

Status: **decisions taken, design to be fleshed out** (2026-11-07, revised 2026-11-14). Part of
[visor panels](README.md). Records the desktop's side of
[ADR 0045](../adr/0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md) as amended by
[ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md).

## Goal

Every instance presents its views in panels, and the visor stage places the panels of all instances
of a launcher on one arc. The desktop scales each instance and moves the keyboard focus between the
views.

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
  receives `ViewEvent::Resized`. The view is then fitted back into its panel with
  `fullscreen_scale` ([fullscreen.rs:30](../../desktop/src/desktop_system/fullscreen.rs#L30),
  [instance_presenter.rs:581](../../desktop/src/instance_presenter.rs#L581),
  [ADR 0014](../adr/0014-fullscreen-mode-is-per-launcher-content-scaling.md)).

## Decisions

### Applications and launchers

- **Every application may present several views.** There is no fixed property that splits
  applications into multi-instance and multi-panel ones. The one-view limit of `present_view` is
  lifted for every instance.
- **A launcher runs any number of instances.** Cmd+T and Shift+Cmd+T always start another one,
  whatever application the launcher starts.
- **The shell still shows one view per instance.** A second view ends the shell with an error, as
  [ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md) already
  says. An application that needs several views therefore runs only in the desktop.
- **A launcher names its application.** An optional `application` field in the desktop
  configuration names the application the launcher starts. Without it, the launcher starts the
  primary application. The field changes the persisted configuration, the
  [KDL plan](../desktop-configuration-kdl-plan.md), and `desktop_describe`, and both security
  documents ([massive](../security.md), [mt](../../../docs/security.md)) are updated in the same
  change.

### Panels on the arc

- **Each instance contributes a group of panels.** The panels come from its views' layout metadata
  ([05](05-layout-stage.md)). An instance with one view contributes one panel.
- **Panels are the visor stage's direct children.** Instances exist only in the target tree. One
  visor layouter lays out the whole arc ([06](06-visor-layouter.md)).
- **A panel is a target.** Its instance and its panel index identify it. If the application changes
  a panel index, the panel moves.
- **The visor stage joins the groups in the order of the instances.** A nested instance's group goes
  after the panel of its parent instance that the parent names
  ([ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md)).
- **Each panel gets its own scene location.** The visor stage moves and turns every panel on its
  own.
- **Panels sit edge to edge.** There is no gap between neighboring panels.
- **The arc places panels of any width and height.** The placement takes the panels' widths instead
  of an index and a count. Equal widths keep today's look.
- **The arc centers on the panel of the focused view.**
- **The center line lies at the launcher's vertical center.** Panels hang from it by their vertical
  offsets ([06](06-visor-layouter.md)), and the arc turns panels only around the vertical axis, so
  the offsets stay valid.
- **Each panel has a title bar.** It shows the title of the panel's remembered view
  ([ADR 0019](../adr/0019-instance-title-bar-is-part-of-the-instance-extent.md) applies per panel).
  The title bar is a node of the stage tree under its panel. A click on it still selects its
  target.

### Scale

- **All panels of an instance share its instance scale.** The instance scale is the largest scale,
  at most 1, at which the panel of the instance's primary view, with its title bar, fits the slot.
  This is the rule that fits an instance into its panel today. Expanding and collapsing never change
  it.
- **The camera cancels the instance scale at the Focus zoom level**, so the focused instance's
  panels are shown 1:1 on screen.
- **Full Screen Mode enlarges the primary view**, so it shrinks all panels of that instance. It
  therefore changes the visor layout, which
  [ADR 0014](../adr/0014-fullscreen-mode-is-per-launcher-content-scaling.md) excluded until
  ADR 0046. Title bars keep their regular size.
- **Nothing visible changes today.** Every current application has one view, so its one panel is
  scaled by the same rule as today.
- **A collapsed visor shows the primary view of the instance that holds the center visor view.**
- **The slot measures the fitted primary view.** So a launcher stays one slot extent ([ADR
  0044](../adr/0044-launchers-present-only-as-visors.md)), and every row of the matrix keeps the
  slot extent ([ADR
  0023](../adr/0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)). Panels may
  extend beyond the slot while the visor is expanded, also vertically.

### Focus and navigation

- **Hit testing walks the stage tree.** It uses the target placements, so a click hits a view at
  the place it moves to, also while it still animates there ([08](08-animation-and-drawing.md)). A
  view without a placement cannot be hit, and focus navigation skips it.
- **The camera frames a launcher by its panels.** The framing in
  [zoom_geometry.rs](../../desktop/src/desktop_system/navigation/zoom_geometry.rs) already reads
  only placements and their yaw. It frames the visor stage's children instead of asking for the
  launcher's instances.
- **Cmd+arrows move to the neighboring view.** In an expanded visor, Cmd+←/→ cross panel edges, and
  Cmd+↑/↓ move within a panel. The neighbor is found by position, whatever instance owns it.
- **Past the outermost view, navigation escapes to the matrix**, as it does today.
- **The visor stage keeps the center visor view.** It reports it on its parent side, so that the
  desktop core focuses it when the user re-enters the launcher. The arc then centers on its panel.
- **The layout focus names a view.** Today it names an instance
  (`LayoutInputs::focused_instance`), which cannot carry the remembered views or the center visor
  view.
- **The layout focus stays in the desktop.** The visor stage receives it from its parent side and
  updates the remembered views from it ([06](06-visor-layouter.md)). The instance never sees it.

## Glossary changes

- Redefine **panel** and **layout focus**, which now names a view. Define **layout metadata**,
  **center line**, **visor stage** and **connector stage**.
- Replace **center visor instance** by **center visor view**, and update **collapsed visor** and
  **non-center visor instance**.
- Update **instance title bar**, which today names the primary view.
- Add **instance scale**.

## Open questions

- How a connector crosses from one panel to the next on the arc, and whether that changes the arc's
  shape, for example to flat panels hinged at their edges ([02](02-connector-shapes.md)).
- Whether `ViewRole` means more than "primary" for an instance with several views.
- Whether `desktop_describe` lists an instance's panels and views.
- How placement visibility ([ADR
  0001](../adr/0001-placement-visibility-drives-interaction-and-fade.md)) applies to several views
  and panels of one instance.
- How a visor gets rows, so that a parent instance can place nested instances below its own panels.
  Panels can have arbitrary heights, which this has to take into account.
