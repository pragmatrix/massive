# Projects and launchers are layout stages, and instances may start nested instances

## Status: proposed

Amends [ADR 0022](0022-presenters-own-the-layout-of-their-targets.md), where it describes how the
desktop lays out projects and launchers. Amends
[ADR 0045](0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md) in its sections on application
kinds, size and scale, and the layout stage.

## Context

[ADR 0045](0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md) puts a layout stage between a
multi-panel application and the desktop. That stage runs in the instance's task. It stacks the
instance's views into panels, slides the panels, and routes and draws the connectors between the
views. The desktop places the panels on the visor's arc and knows nothing about connectors.

The desktop lays out its projects and launchers itself. Each of them has a presenter that implements
`TargetLayout` ([ADR 0022](0022-presenters-own-the-layout-of-their-targets.md)). The launcher's
presenter places the visor's arc
([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs)), and the project's
presenter places the matrix ([project_presenter.rs](../../desktop/src/projects/project_presenter.rs)).
Both read what they need from the desktop through a `LayoutContext`, which holds the hierarchy, the
configuration and the focused instance.

The idea came up to apply the layout stage to the whole desktop. The desktop would become "just an
instance" that can start nested instances, with a layout stage between it and the shell. Each
project, or even each launcher, could then be an instance of its own. A launcher instance would
separate the visor layout from the desktop. A project instance would separate the matrix layout from
the desktop.

Three facts weigh against making every level an instance.

- Several parts of the desktop span every level. The camera zooms relative to the focused target
  ([ADR 0018](0018-zoom-is-a-level-relative-to-the-focused-target.md)). Keys are delivered from the
  top down along the focus path
  ([ADR 0020](0020-keys-are-delivered-top-down-and-the-consumer-is-the-input-focus.md)). The
  selection spans slots ([ADR 0025](0025-slots-can-be-selected-beside-the-keyboard-focus.md)), and
  hit testing spans the whole window.
- `MoveSlot` moves a slot into another project's matrix, and `Group` and `Ungroup` move slots into a
  new project or back into its parent
  ([ADR 0011](0011-desktop-is-a-fractal-of-projects-in-slots.md),
  [ADR 0026](0026-grouping-wraps-the-selected-slots-into-a-project-in-place.md)).
- The desktop draws into its own window, and the shell hosts it as its one instance.
  [ADR 0039](0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md) rejected letting
  the desktop draw through a view, because a view has no camera, no native full screen and no
  background, and the resize-synchronous renderer would be lost.

Two gaps in ADR 0045 also shaped this decision. A connector between two panels crosses a panel edge
on the arc, and only the desktop knows the arc. A connector between the views of two instances has
no place at all, because each layout stage sees only its own instance.

## Decision

### Terms

- The **host** keeps its meaning from the glossary. It is the shell or the desktop. Only the host
  runs instances, keeps their list, routes their events and shows their views.
- An instance may **start nested instances**. It is their **parent instance**, and they are its
  **nested instances**. The parent instance does not host them. It owns their lifetime and decides
  where they appear.
- A **layout stage** lays out the children of one level of the desktop. It sits between that
  level's parent side and its child side.

### Projects and launchers are layout stages, not instances

- A project is a **matrix stage** and a launcher is a **visor stage**. Both run in the desktop's
  task. Neither is an instance.
- Stages in one task call each other directly, with no channels between them. One layout pass
  measures and places every stage, so the camera and hit testing read placements from the same pass.
- The desktop core keeps the camera, the focus path, the selection, hit testing, the configuration
  aggregate with its persistence, and MCP. It reads only the placements that stages produce, so it
  does not know what a matrix or a visor is.
- `MoveSlot`, `Group` and `Ungroup` re-parent stages. No running instance changes its parent.
- The shell still hosts the desktop as its one instance, and no stage sits between them.
  [ADR 0039](0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md) is unchanged.
  The desktop owns the only camera, and everything below it is framed by that camera.

### A stage learns everything from its two sides

- A stage keeps state of its own. Examples are the view that was focused most recently, whether a
  visor is expanded, and how far an animation has run.
- Everything else reaches the stage from its two sides while it runs. It never reads the desktop's
  aggregate, its hierarchy or a `LayoutContext`.
- The parent side delivers two kinds of input.
  - The topology comes from the desktop core, which owns the aggregate. A stage receives only its own
    direct children, for example a matrix stage receives its slot positions and the child in each.
    The core sends this list only to the stages whose children changed. A `MoveSlot` between two
    projects therefore sends two short lists, and a change of focus sends none.
  - The layout values come from the parent stage. They are the stage's placement, its presentation
    scale ([ADR 0023](0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)), the
    layout focus, and the animation ticks.
- To its parent side, a stage reports its measured extent.
- From its child side, a stage receives the children's sizes and, in a visor, the layout metadata of
  the views. To its child side, it sends one placement per child.
- A value that siblings share is settled by their parent stage. The shared column count
  ([ADR 0035](0035-sibling-projects-share-a-column-count.md)) and the shared project scale
  ([ADR 0015](0015-nested-projects-share-a-width-derived-presentation-scale.md)) are examples. Each
  child reports what it needs, and the parent sends the shared value down. Siblings never read each
  other.
- Stages receive no keys. The keys that a launcher or a project handles today start instances, remove
  slots, zoom and select, so they are commands of their targets, not layout. They stay with the
  targets in the core.

### A pure layouter computes each stage's geometry

- Each stage computes its geometry with a **layouter**. A layouter is a pure function from the
  stage's inputs and state to placements, as ADR 0045 already defines it.
- A layouter is not incremental. It recomputes one stage in full, and a stage has only a few
  children. The stage remembers its last inputs and outputs. It runs the layouter only when an input
  changed, and it reports to its parent side only when its extent changed.
- Between stages, the layout stays incremental as it is today. Measuring a target marks its parent
  for measuring only when its size changed
  ([layout_effects.rs](../../desktop/src/desktop_system/layout_effects.rs),
  [ADR 0043](0043-effects-are-ordered-by-what-they-read-and-marked-through-the-transaction.md)).
- Each stage animates its own placements. Stages share the animation code through a library.

### The `Stage` trait is open

- A `Stage` trait in `massive-applications` describes a stage. The matrix stage and the visor stage
  implement it. The trait comes first, so that every stage is designed against the same constraint
  from the start.
- A stage's inputs and outputs are plain data, so that they can cross the boundary to an instance
  that runs out of process.
- An application may bring a stage of its own. That stage runs only in the application's own task,
  and it computes the layout metadata of the application's views.
- The side that holds children picks the stage between itself and them. A project picks the matrix
  stage, and a launcher picks the visor stage.

### Every instance contributes panels to its launcher's visor

- An application is no longer either multi-instance or multi-panel. That fixed property of ADR 0045
  is removed. A launcher runs any number of instances of any application, and Cmd+T and Shift+Cmd+T
  always start another one.
- Each view carries **layout metadata**. It consists of a panel index, a stack index, the view's
  ports, and its outgoing connectors. The indices count within the view's instance, so panel 0 of
  one instance and panel 0 of another are different panels.
- An instance with one view declares nothing. Its primary view defaults to panel 0, stack 0, so the
  applications that run today do not change. Any other view stays hidden until its instance gives it
  indices.
- The visor stage joins the panels of all its instances on one arc, one group of panels per
  instance, in the order of the instances and without gaps.
- The visor stage does everything that ADR 0045 gave to the layout stage in the instance. It stacks
  each panel's views, keeps the remembered view of each panel, slides the panels along the center
  line, routes the connectors, and draws and animates views and connectors together. All of this
  happens in the desktop's task and in one frame. The layout focus therefore no longer crosses into
  the instance, and the sliding starts together with the centering of the arc by construction.
- The visor stage keeps the center visor view. It reports that view on its parent side, so that the
  core can focus it when the user re-enters the launcher.
- A collapsed visor shows the primary view of the instance that holds the center visor view. Only a
  primary view has to fit the slot, as in ADR 0045.

### One scale per instance

- All panels of an instance share its **instance scale**. The instance scale is the largest scale,
  at most 1, at which the instance's primary view fits the slot.
- At the Focus zoom level, the camera comes close enough to cancel the instance scale, so the
  focused instance's panels are shown 1:1 on screen.
- Full Screen Mode enlarges the primary view
  ([ADR 0014](0014-fullscreen-mode-is-per-launcher-content-scaling.md)), so it shrinks all panels of
  that instance.
- Nothing visible changes today. Every current application has one view at the default panel size,
  which gets the scale 1, and Full Screen Mode already fits that view back into its panel with the
  same rule.

### Connectors may join views of different instances

- A connector end names a view by its instance and its view, and optionally one of the view's ports.
- Both ends must be views in the same visor. A connector therefore joins views of instances of one
  launcher.
- The visor stage routes all connectors, the ones within one instance as well as the ones between
  instances. It knows the arc, so it also routes a connector across a panel edge on the arc.

### An instance may start nested instances

- An application declares as a fixed property whether it starts nested instances. The property sits
  next to its draw target. The desktop refuses a start request from an application that does not
  declare it.
- The parent instance asks for a nested instance through its `InstanceContext`. This API and the
  property live in `massive-applications`. The registry of applications, the start of the instance
  and the checks stay with the desktop.
- A nested instance must be an application that the desktop's registry knows. It inherits its parent
  instance's MCP root ([ADR 0036](0036-an-mcp-root-exposes-a-project-subtree-to-agents.md)), so it
  can reach no more than its parent can.
- The desktop knows every nested instance. Its views are ordinary desktop targets in the focus path,
  in hit testing and in MCP. `desktop_describe` lists a nested instance under its parent instance.
- A nested instance does not know that it is nested. It creates its views, declares their layout
  metadata and draws straight to the renderer, like every other instance. Keys go straight to it.
- A nested instance appears in its parent instance's visor and brings its own group of panels. The
  parent instance names after which of its own panels that group goes. It names the place in the
  start request and may change it later. Without a place, the group follows the parent's group.
- The parent instance owns the lifetime of its nested instances. It can stop one. When the parent
  instance ends, the desktop ends its nested instances. When a nested instance ends, its parent
  instance receives an event.
- The shell runs an application that starts nested instances. It answers every start request with an
  error, so such an application still runs on its own, and it decides how to do without its nested
  instances.

## Considered options

- **An instance per project or per launcher.** Every level would be isolated, could crash on its own
  and could later run out of process. Every frame would pass through one more message hop per
  nesting level. Delivering a key from the top down would need a request and a reply per level.
  Placements could reach the level above a frame late. The camera, the focus path and hit testing
  would become protocols between instances. `MoveSlot`, `Group` and `Ungroup` would move running
  instances from one parent to another.
- **Stages that are designed to become instances later.** The open `Stage` trait with plain data
  covers most of this option anyway.
- **A window stage in the shell, with the desktop drawing into views.** A desktop could then run
  inside another desktop. It is the option that ADR 0039 rejected, and a camera inside another camera
  would need a design of its own.
- **Only the desktop may start instances.** No application needs nested instances today. The user
  chose to make them available to applications.
- **A nested instance hidden behind its parent.** The parent instance would receive the nested
  instance's events and pass them on. Every level would add a request and a reply per key. The
  desktop could not enforce the nested instance's MCP root, and it would draw the content of an
  instance it does not know.
- **The parent instance forwards the nested instance's scene content.** Every nesting level would
  add a hop to every frame. The [out-of-process plan](../out-of-process-applications-plan.md) sets a
  performance bar that this would endanger for a terminal under heavy output.
- **The parent instance places the nested instance's views in its own panels.** The parent could
  stack its own views and the nested ones in one panel. Each nested view would need a round trip
  through the parent before it could appear, and the nested instance would be the one instance that
  does not lay out its own panels.
- **The layout stage in the instance routes the connectors**, as ADR 0045 planned. It cannot route a
  connector to a view of another instance, and it cannot route across a panel edge on the arc,
  because it does not know the arc.
- **Any instance may start nested instances.** Without a declaration, any running application could
  start other applications on the desktop, including an application that acts on input from a remote
  machine. That widens the trust model beyond what [security.md](../security.md) covers.
- **The hosted application, or the configuration, picks the stages above it.** If the application
  picked them, it would take decisions that belong to the desktop policy
  ([ADR 0029](0029-exceptional-desktop-behaviors-are-decided-by-the-desktop-policy.md)). If the
  configuration picked them, it would bring back the mode that
  [ADR 0044](0044-launchers-present-only-as-visors.md) removed, without a second stage that serves
  someone.
- **Layout and presentation as two stages in series.** One presentation stage would animate every
  level. The placement it receives would have to carry transitions and visibility, and the layout
  focus would pass through two stages per level.
- **A closed set of stages.** Applications bring their own stages, so the set cannot be closed.
- **The scale depends on the panel count.** An instance with one panel would be scaled to fit, and an
  instance with several panels would be shown 1:1 while expanded. An instance would change its scale
  the moment it opens a second panel.
- **Every expanded panel at 1:1.** A full-screen terminal would cover its neighboring slots at the
  Slot, Row and Project zoom levels, which ADR 0045 rejected.
- **The shell refuses an application that starts nested instances.** It would fail fast, but such an
  application could never run on its own.
- **The shell runs nested instances, one window each.** The application would behave the same under
  both hosts. The shell would need a registry, starts and event routing across windows, the check of
  the declared property would exist twice, and the MCP root has no meaning without a desktop socket.
- **A separate channel for layout metadata and requests to the parent.** Scene changes would stay
  apart from everything else. Layout metadata must take effect in the same frame as the scene changes
  that it positions, so a second channel would need sequence numbers or fences to restore the order
  of the one channel.

## Consequences

- The presenters' layout of ADR 0022 becomes the layout of stages. Every value that a presenter now
  reads through `LayoutContext` needs a source on one of the stage's two sides. Finding these sources
  is most of the work of the matrix stage.
- ADR 0045 changes in three sections. Its two kinds of applications are gone. Its two ways of scaling
  become the one instance scale. Its layout stage in the instance becomes the visor stage in the
  desktop, and connector and port metadata now cross into the desktop.
- ADR 0045's open questions get answers. A connector crosses the arc because the visor stage routes
  it. Full Screen Mode shrinks every panel of its instance. A view without indices stays hidden,
  except the primary view, which defaults to panel 0, stack 0.
- The visor panels documents [03](../visor-panels/03-multi-panel-instances.md),
  [05](../visor-panels/05-layout-stage.md), [07](../visor-panels/07-connector-routing.md) and
  [08](../visor-panels/08-presentation-layer.md) are revised before they are implemented. Routing and
  presentation move into the visor stage.
- Connector routing runs in the desktop's task. A large connector graph could therefore slow the
  desktop and not only one instance. Its cost is measured before it is optimized.
- Both security documents change in the change that introduces nested instances. They gain the
  declared property, the check that only a declaring application may start nested instances, and the
  rule that a nested instance inherits its parent's MCP root.
- `desktop_describe` lists nested instances under their parent instance.
- The glossary gains layout stage, matrix stage, visor stage, layout metadata, parent instance,
  nested instance and instance scale in the changes that introduce them. Layout stage is never
  shortened to stage alone in the glossary, because the lifetime stage
  ([ADR 0040](0040-a-lifetime-stage-before-the-renderer-decides-when-scene-objects-end.md)) is a
  different thing. The terms multi-instance application and multi-panel application of ADR 0045 are
  not introduced.
- The work proceeds in three steps. The visor stage is taken out of the launcher's presenter first.
  The matrix stage follows. Nested instances come last.
- The channel from an instance to the desktop stays as it is, with scene changes, view changes and
  requests interleaved in one ordered submission. A later change could structure each submission
  into a scene part and a part for the desktop, and send requests as messages of their own on the
  same channel. That would separate the types without losing the order.

## Open questions

- The message types of the `Stage` trait.
- Whether a stage will need a say about key presses.
- How a visor gets rows, so that a parent instance can place nested instances below its own panels.
  Panels can have arbitrary heights, which this has to take into account.
- Whether connectors may join views of different launchers.
- Whether a nested instance may itself start nested instances.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- Whether mt can register a stage that runs in the desktop's task.
- Whether `desktop_describe` lists an instance's panels and views.
