# A visor is an arc of panels, and a layout stage lays out the views in them

## Status: proposed

Amended by [ADR 0046](0046-projects-and-launchers-are-layout-stages.md) (proposed). Applications are
no longer multi-instance or multi-panel, every instance shares one instance scale across its panels,
and the visor stage in the desktop takes over the layout stage's work, connectors included.

## Context

Every launcher presents its instances as a visor
([ADR 0044](0044-launchers-present-only-as-visors.md)). A visor places the instances of a launcher
on an arc around its center visor instance. When the keyboard focus leaves the launcher, the visor
collapses to that instance ([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs),
[visor_layout.rs](../../desktop/src/projects/visor_layout.rs)).

The view columns plan, now revised as [visor panels](../visor-panels/README.md), lets one instance
present many views, arranged in view columns and joined by connectors. In that plan the view
columns lie flat inside the instance's one panel, which the desktop scales to fit, and the desktop
knows nothing about columns.

So both designs arrange content horizontally around a focus, each in its own way. This decision
unifies them, so that one view column becomes one visor panel. None of the view columns plan has
been implemented, so the decision changes only plans.

## Decision

### Panels

- A **panel** is one unit of a visor. It holds a vertical stack of one or more views. The term
  replaces the plan's "view column". It also replaces the plan's earlier meaning of "panel", which
  was the rect that holds all of an instance's content.
- A launcher's visor is one arc of panels. Panels sit side by side with no gap between them.
- The arc places panels of any width and any height. Today's arc assumes panels of equal width, so
  its placement becomes generic.

### Applications are multi-instance or multi-panel

- Every application is either multi-instance or multi-panel, and the application states which as a
  fixed property, next to its draw target.
- An instance of a **multi-instance application** presents one panel. This is how every application
  behaves today.
- A launcher of a **multi-panel application** runs exactly one instance, and that instance presents
  any number of panels. While the instance runs, the launcher refuses to start another one, either
  with Cmd+T or as an assistant with Shift+Cmd+T.
- The shell shows one view per instance, so it refuses to run a multi-panel application.
- A launcher names the application it starts in an optional `application` field. Without the field,
  it starts the primary application. Today every launcher starts the primary application.

### Size and scale

The two kinds of launchers scale their content in different ways.

In both kinds, the slot measures the primary view fitted into the slot. So every row of the matrix
keeps the slot extent
([ADR 0023](0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)).

**A multi-instance launcher scales as it does today.**

- Each instance is shown at its **instance scale**. The instance scale is the largest scale at which
  the instance's primary view fits the slot, and it never enlarges.
- The instance scale does not depend on the focus. Expanding the visor shows the other instances,
  and collapsing it hides them, but neither changes a scale.
- At the Focus zoom level, the camera comes close enough to cancel the instance scale, so the
  focused instance is shown at 1:1 on screen.
- Full Screen Mode stays as it is
  ([ADR 0014](0014-fullscreen-mode-is-per-launcher-content-scaling.md)). The desktop decides it and
  gives the primary view the size of the window, and the instance only receives the new size of its
  view. The instance scale fits that window-sized view into the slot. So a full-screen instance stays
  shrunk into its slot at every zoom level except Focus.

**A multi-panel launcher changes its scale when the visor expands or collapses.**

- The primary view can have any size.
- While the launcher does not hold the layout focus, the visor is collapsed. It then shows only the
  primary view, and the desktop scales that view to fit the slot.
- While the launcher holds the layout focus, the visor is expanded. Every panel is then shown at 1:1
  and may extend beyond the slot.
- The change between the two scales is animated like other structural changes.

### Navigation and focus

- In an expanded visor, Cmd+arrows move the keyboard focus to the neighboring view by position. Left
  and right cross panel edges, and the views do not need to belong to the same instance.
- The arc centers on the panel that holds the focused view.
- When no neighboring view exists in the launcher, navigation escapes to the matrix, as it does
  today.
- The desktop remembers one **center visor view** per launcher. It generalizes the center visor
  instance from instances to views. Re-entering the launcher focuses it, and the arc centers on its
  panel.
- The focus companion of a view is the view that Cmd+← would move to. The desktop derives it from
  the positions it knows, so the instance does not name companions.
- Each panel has its own title bar. The desktop knows nothing about the stage's remembered views,
  which the layout stage section defines. So the stage names, for each panel, the view whose title
  the panel's title bar shows, and it names the panel's remembered view.

### The layout stage

- A **layout stage** sits between a multi-panel application and the desktop. It lays out the views
  and makes them slide. The application knows only which stage it uses.
- For each view, the application supplies a **panel index** and a **stack index**. The panel index
  says which panel the view belongs to, and the stack index says where in the panel's stack it sits.
  The application also declares connectors and ports. It never positions a view.
- The stage remembers one view per panel, the panel's **remembered view**. The desktop sends the
  stage the view that holds the layout focus, and the stage updates the remembered view of that
  view's panel. The application never sees this event.
- The stage follows the layout focus, not the keyboard focus. The layout focus waits while a mouse
  button is pressed, so a click never slides a panel under the pointer
  ([ADR 0002](0002-keyboard-focus-changes-are-command-mediated.md)). The stage's sliding also starts
  together with the desktop's centering of the arc.
- Every panel slides vertically so that the center of its remembered view lies on a **center line**
  at y = 0. The focused view is the remembered view of its panel, so it lies on the center line too.
  The desktop places the center line at the launcher's vertical center. This sliding strategy is a
  parameter of the stage's layouter, and it is the only strategy for now.
- The stage sends only relative coordinates. A panel's vertical offset is relative to the center
  line, and a view's position is relative to its panel. Horizontally, the stage sends only the order
  of the panels and their widths, and the desktop places them on the arc.
- A panel is as wide as its widest view, plus the margins its connectors need. A narrower view sits
  centered in its panel.
- A **connector** joins two views and can cross panel edges. Connectors do not move views. They
  decide only how routes are drawn and how wide the margins are.

## Considered options

- **An arc per instance inside its slot.** The launcher's arc of instances would stay, so a launcher
  with several multi-panel instances would show arcs inside an arc. That keeps two layouts, two arc
  cameras, and two ways to collapse.
- **View columns flat in one panel, scaled to fit.** This was the original plan. It keeps a second
  horizontal arrangement next to the visor, which this decision removes.
- **Panels of a fixed height.** A panel's height comes from its stack of views. Only the primary
  view has to fit the slot.
- **One way of scaling for both kinds of launchers.** If both kinds showed everything at 1:1 when
  expanded, a full-screen instance in a multi-instance launcher would be as large as the window on
  the desktop. It would then cover its neighboring slots at the Slot, Row, and Project zoom levels,
  where today it stays shrunk into its slot. If both kinds kept one fixed scale instead, the panels
  of a multi-panel instance would stay shrunk to the scale of its primary view while expanded.
- **Multi-panel as a launcher setting, or decided at runtime.** The application knows which kind it
  is. A launcher setting could disagree with the application. A decision at runtime would make Cmd+T
  depend on what happens to run.
- **The desktop remembers a view per panel.** Navigation moves by position, so only the stage's
  sliding needs that memory.
- **The stage slides on the keyboard focus.** A click would then slide panels under the pointer, and
  the vertical slide would start before the arc recenters.
- **Panels align along their connectors**, as the view columns plan's column layouter did. A panel
  would then slide so that the views connected to the focused view sit beside it. This is deferred
  for now in favor of the center line, which does not depend on connectors.

## Consequences

- Instances that run today do not change visibly, because every application today is
  multi-instance.
- [ADR 0019](0019-instance-title-bar-is-part-of-the-instance-extent.md) applies to each panel
  instead of to each instance.
- The `application` field changes the persisted desktop configuration, the
  [KDL plan](../desktop-configuration-kdl-plan.md), and `desktop_describe`. Both security documents
  are updated in the same change.
- A launcher still measures exactly one slot extent
  ([ADR 0044](0044-launchers-present-only-as-visors.md)). The scaling of a multi-panel launcher
  applies to the primary view inside the launcher, not to the launcher.
- The view columns plan is revised as the [visor panels](../visor-panels/README.md) plan. Its
  pixel-perfect experiment is dropped, because an expanded multi-panel launcher shows its panels at
  1:1 and the camera has no scale to cancel. Its column layouter no longer assigns columns or orders
  views, because the application supplies panel and stack indices. The desktop derives the focus
  companions, and the instance no longer sends them.
- The glossary changes the terms panel, center visor instance, and collapsed visor in the change
  that introduces them. The center visor view replaces the center visor instance. The glossary also
  adds the instance scale, which names how a multi-instance launcher scales today.

## Open questions

- How a connector crosses from one panel to the next on the arc. It may need to bend smoothly along
  the arc. The answer can change the arc's own shape, for example flat panels hinged at their edges,
  or panels on a circle as today.
- How Full Screen Mode works in a multi-panel launcher. Showing a window-sized primary view at 1:1
  would make it cover its neighboring slots while the visor is expanded.
- Whether users can configure the sliding strategy, and whether panels align along their connectors
  later.
- Whether `desktop_describe` lists an instance's panels and views.
- What happens to a view whose application has not yet given it a panel index and a stack index.
- How views appear and disappear.
