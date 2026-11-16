# Projects and launchers are layout stages, and instances may start nested instances

## Status: proposed

Amends [ADR 0022](archive/0022-presenters-own-the-layout-of-their-targets.md), where it describes how the
desktop lays out projects and launchers. Amends
[ADR 0045](0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md) in its sections on application
kinds, size and scale, navigation and focus, the layout stage, and its consequences. Amends
[ADR 0014](archive/0014-fullscreen-mode-is-per-launcher-content-scaling.md), because Full Screen Mode now
changes the visor layout.

## Context

[ADR 0045](0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md) puts a layout stage between a
multi-panel application and the desktop. That stage runs in the instance's task. It stacks the
instance's views into panels, slides the panels, and routes and draws the connectors between the
views. The desktop places the panels on the visor's arc and knows nothing about connectors.

The desktop lays out its projects and launchers itself. Each of them has a presenter that implements
`TargetLayout` ([ADR 0022](archive/0022-presenters-own-the-layout-of-their-targets.md)). The launcher's
presenter places the visor's arc
([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs)), and the project's
presenter places the matrix
([project_presenter.rs](../../desktop/src/projects/project_presenter.rs)). Both read what they need
from the desktop through a `LayoutContext`
([target_layout.rs](../../desktop/src/desktop_system/target_layout.rs)). It holds the hierarchy, the
configuration, the shared column counts, the default panel size, the window size, the focused
instance and the title bar metrics.

One tree, `OrderedHierarchy<DesktopTarget>`, serves five jobs at once. It holds the structure of
projects, launchers, instances and views. It carries the focus, the key path of
[ADR 0020](archive/0020-keys-are-delivered-top-down-and-the-consumer-is-the-input-focus.md), the zoom chain
and Esc. The layout measures and places along it. Hit testing and coordinate mapping walk it. Each
presenter's scene location hangs under its parent's location along it.

The idea came up to apply the layout stage to the whole desktop. The desktop would become "just an
instance" that can start nested instances, with a layout stage between it and the shell. Each
project, or even each launcher, could then be an instance of its own. A launcher instance would
separate the visor layout from the desktop. A project instance would separate the matrix layout from
the desktop.

Three facts weigh against making every level an instance.

- Several parts of the desktop span every level. The camera zooms relative to the focused target
  ([ADR 0018](archive/0018-zoom-is-a-level-relative-to-the-focused-target.md)). Keys are delivered from the
  top down along the focus path, and outer levels get the first refusal (ADR 0020). The selection
  spans slots ([ADR 0025](archive/0025-slots-can-be-selected-beside-the-keyboard-focus.md)), and hit testing
  spans the whole window.
- `MoveSlot` moves a slot into another project's matrix, and `Group` and `Ungroup` move slots into a
  new project or back into its parent
  ([ADR 0011](0011-desktop-is-a-fractal-of-projects-in-slots.md),
  [ADR 0026](archive/0026-grouping-wraps-the-selected-slots-into-a-project-in-place.md)).
- The desktop draws into its own window, and the shell hosts it as its one instance.
  [ADR 0039](archive/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md) rejected letting
  the desktop draw through a view, because a view has no camera, no native full screen and no
  background, and the resize-synchronous renderer would be lost.

A gap in ADR 0045 also shaped this decision. A connector between two panels crosses a panel edge on
the arc, and only the desktop knows the arc.

## Decision

### Terms

- The **host** keeps its meaning from the glossary. It is the shell or the desktop. Only the host
  runs instances, keeps their list, routes their events and shows their views.
- An instance may **start nested instances**. It is their **parent instance**, and they are its
  **nested instances**. The parent instance does not host them. It owns their lifetime and decides
  where they appear.
- The **desktop core** is the part of the desktop that is not a stage. It holds the camera, the
  focus, the selection, hit testing, the configuration aggregate with its persistence, and MCP.
- The **target tree** is today's `OrderedHierarchy<DesktopTarget>`. The **stage tree** is a second
  tree, which holds the layout.
- A **stage** is a node of the stage tree that has children. A **layout stage** is a stage that lays
  out its children with a layouter.
- A stage's **parent side** faces the root of the stage tree. Its **child side** faces its
  children.
- A **layouter** is the pure function that a layout stage calls. It takes the stage's state, inputs
  and events, and it returns the stage's new state and its output.
- A view's **layout metadata** is what the view declares about its place. It consists of a panel
  index, a stack index, the view's ports, and its outgoing connectors.
- A **stack gap** is the space between two views stacked in a panel. A **panel margin** is the space
  beside a panel's views, on its left or its right. A **connector lane** is the room that one
  connector takes up where it passes through a stack gap or a panel margin.
- A **connector route** is the path drawn for one connector, from its source end to its target end
  ([visor panels 07](../visor-panels/07-connector-routing.md)). Its **route topology** says which
  stack gap or which way around a stack it takes in every panel it passes, and which connector lane
  it uses. The route topology needs no positions. Its **route geometry** is the exact lines and
  arcs, computed from the route topology and the current positions of ports, views and panels.

### Projects and launchers are layout stages, not instances

- A project is a **matrix stage**, which is a layout stage with the matrix layouter. A launcher is a
  **visor stage**, which is a layout stage with the visor layouter. Both run in the desktop's task.
  Neither is an instance.
- Stages in one task call each other directly, with no channels between them. One layout pass
  measures and places every stage, so the camera and hit testing read placements from the same pass.
- The desktop core reads only the placements that stages produce, so it does not know what a matrix
  or a visor is.
- `MoveSlot`, `Group` and `Ungroup` re-parent stages. No running instance changes its parent.
- The shell still hosts the desktop as its one instance, and no stage sits between them.
  [ADR 0039](archive/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md) is unchanged.
  The desktop owns the only camera, and everything below it is framed by that camera.

### Two trees: the target tree and the stage tree

- The target tree keeps the structure, the focus, the key path, the zoom chain, Esc, MCP and the
  lifetime of instances. It holds projects, launchers, instances and views, as today.
- The stage tree holds the layout, hit testing, coordinate mapping and scene parenting. It holds the
  stages, the panels, the title bars, the project headers and the placed views.
- Every node of the stage tree carries the target it shows. A hit test walks the stage tree and
  returns the target of the node it hits, such as a view, a title bar or a project header.
- The target tree is the source of truth. The stage tree is derived from it and from the layout
  metadata, and it is never edited on its own.
- Each stage owns only the list of its direct children. The stages live in a map keyed by their
  target, as the presenters in `Aggregates` do today.
- The target tree changes in three places: `add`, `remove` and `reparent`. Each of them marks a
  **`SyncChildren`** effect for every parent whose children changed. A reparent marks the old and
  the new parent.
- `SyncChildren(parent)` reads the parent's children from the target tree and the layout metadata,
  and it gives the parent's stage its new child list. The stage compares the list with its last one
  and marks `Measure` only when something changed. The effect reads the target tree, so it runs
  after the structural changes and before `Measure` ([ADR
  0043](archive/0043-effects-are-ordered-by-what-they-read-and-marked-through-the-transaction.md)).
- The rules where the two trees differ live in `SyncChildren` alone. A visor stage's children are
  panels, not instances. A connector stage sits above every visor stage. A nested instance's panels
  join its parent's visor.
- A view without a placement is not in the stage tree. It cannot be hit, and focus navigation skips
  it.

### A stage learns everything from its two sides

- A stage keeps state of its own. Examples are the view that was focused most recently, whether a
  visor is expanded, and how far an animation has run.
- Everything else reaches the stage from its two sides while it runs. It never reads the desktop's
  aggregate, the target tree or a `LayoutContext`.
- From its parent side, a stage receives these inputs.
  - Its direct children, through `SyncChildren`, only when they change. A `MoveSlot` between two
    projects therefore sends two short lists, and a change of focus sends none.
  - Its placement and its presentation scale
    ([ADR 0023](archive/0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)), from its
    parent stage.
  - The layout focus and the animation ticks.
  - Values that its parent settles for it. A visor stage receives the connector lanes that its
    connector stage needs.
  - The window size and the default panel size, which the layout of Full Screen Mode needs.
- To its parent side, a stage reports its measured extent and what its layouter reports. A visor
  stage reports its center visor view, its panel structure, and its remembered views.
- From its child side, a stage receives the children's sizes and, in a visor, the layout metadata of
  the views. To its child side, it sends one placement per child.
- A value that siblings share is settled by their parent stage. The shared column count
  ([ADR 0035](0035-sibling-projects-share-a-column-count.md)) and the shared project scale
  ([ADR 0015](archive/0015-nested-projects-share-a-width-derived-presentation-scale.md)) are examples. Each
  child reports what it needs, and the parent sends the shared value down. Siblings never read each
  other.
- Stages receive no keys. The keys that a launcher or a project handles today start instances,
  remove slots, zoom and select, so they are commands of their targets, not layout. They stay with
  the targets in the desktop core.

### One generic layout stage, and a layouter per kind

- One generic `LayoutStage<L: Layouter>` implements every layout stage. It handles the protocol with
  both sides, skips runs whose inputs did not change, and sets its children's placements. These
  parts exist once, for every kind of stage.
- What differs between the kinds is only the layouter and the type of its state. The matrix layouter
  and the visor layouter are the first two. A new arrangement, such as rows in a visor, is a new
  layouter in the desktop and not a new stage.
- A layouter is pure, as ADR 0045 already defines it. Its output consists of the placements of the
  children with their visibility, plain decoration, and what the stage reports to its parent side.
- Plain decoration is only what cannot be hit, such as a visor background. A title bar or a project
  header can be clicked to select its target
  ([ADR 0019](archive/0019-instance-title-bar-is-part-of-the-instance-extent.md)), so it is a node of the
  stage tree with its target, placed like any other child.
- A layouter is not incremental. It recomputes one stage in full, and a stage has only a few
  children. The stage remembers its last inputs and outputs. It runs the layouter only when an input
  changed, and it reports to its parent side only when its extent changed.
- Between stages, the layout stays incremental as it is today. Measuring a target marks its parent
  for measuring only when its size changed
  ([layout_effects.rs](../../desktop/src/desktop_system/layout_effects.rs), ADR 0043).

### Every node animates its own placement

- Every node of the stage tree owns its scene location. When its parent sets its placement, the node
  animates towards it, as `apply_layout(layout, animate)` does today
  ([launcher_presenter.rs](../../desktop/src/projects/launcher_presenter.rs)).
- One generic **placed node** serves stages, panels, title bars, project headers and views. It holds
  a scene location, a movement and a fade.
- While a placed node moves, it reports its current value every tick. The desktop core composes the
  current values once per tick into positions relative to each connector stage and passes them to
  it. The views sit two levels below a visor stage, under their panels, so even the connector stage
  directly above a visor stage needs this composition.

### The `Stage` and `Layouter` traits

- A `Stage` trait is the interface through which the desktop core holds the stage tree, whatever the
  kinds of its stages. `LayoutStage<L>` and the connector stage implement it.
- A `Layouter` trait is the extension point for new arrangements.
- Both traits come first, so that every stage is designed against the same constraint from the
  start.
- Both traits live in `massive-desktop`, because only the desktop implements and calls them.
- The types of the layout metadata live in `massive-applications`, because applications declare
  them. They are plain data, so they also cross the boundary to an instance that runs out of
  process. `massive-applications` gains no dependency on `massive-layout`.
- Applications bring only layout metadata, never a stage or a layouter.
- The side that holds children picks the stage between itself and them. A project picks the matrix
  stage, and a launcher picks the visor stage.

### Every instance contributes panels to its launcher's visor

- An application is no longer either multi-instance or multi-panel. That fixed property of ADR 0045
  is removed. A launcher runs any number of instances of any application, and Cmd+T and Shift+Cmd+T
  always start another one.
- The indices of the layout metadata count within the view's instance, so panel 0 of one instance
  and panel 0 of another are different panels.
- An instance with one view declares nothing. Its primary view defaults to panel 0, stack 0, so the
  applications that run today do not change. Any other view stays hidden until its instance gives it
  indices.
- Panels are the visor stage's direct children. A panel is a target that its instance and its panel
  index identify. If the application changes a panel index, the panel moves.
- One visor layouter lays out the whole arc. It joins the panels of all instances of the launcher,
  one group of panels per instance, in the order of the instances and without gaps.
- The visor layouter takes over what ADR 0045 gave to the layout stage in the instance, apart from
  the connectors. It stacks each panel's views, keeps the remembered view of each panel, and slides
  the panels along the center line. All of this happens in the desktop's task and in one frame. The
  layout focus therefore no longer crosses into the instance, and the sliding starts together with
  the centering of the arc by construction.
- The layout focus now names a view, not an instance. Today's layout focus names an instance
  (`LayoutInputs::focused_instance`), which cannot carry the remembered views or the center visor
  view.
- The visor stage reports its **panel structure** to its parent side. The panel structure is the
  order of the panels, the stack of views in each, the connectors, and the port rects, without
  sizes. A change to a port rect marks the visor stage again.
- The visor stage reports the remembered views apart from the panel structure. A focus move then
  changes only which connectors are emphasized, not the route topology.
- From its parent side, the visor stage receives how many connector lanes each stack gap and each
  panel margin needs. Its layouter sizes them for these lanes.
- The visor stage keeps the center visor view. It reports that view on its parent side, so that the
  desktop core can focus it when the user re-enters the launcher.
- A collapsed visor shows the primary view of the instance that holds the center visor view.
- Each panel has a title bar that shows the title of the panel's remembered view. The title bar is a
  part of that view, so a click on it selects the view. Today's title bar is a part of its instance,
  which no longer identifies one bar once an instance has several panels. The glossary's "part"
  changes with this ADR, because it names only parts of a project or an instance today.
- The camera frames a launcher by the placements of the visor stage's children, which are panels.
  The visor-specific framing in
  [zoom_geometry.rs](../../desktop/src/desktop_system/navigation/zoom_geometry.rs) already reads
  only placements, and it stops asking the target tree for the launcher's instances.

### One scale per instance

- All panels of an instance share its **instance scale**. The instance scale is the largest scale,
  at most 1, at which the panel of the instance's primary view, with its title bar, fits the slot.
  This is the rule that fits an instance into its panel today
  ([instance_presenter.rs](../../desktop/src/instance_presenter.rs)).
- At the Focus zoom level, the camera comes close enough to cancel the instance scale, so the
  focused instance's panels are shown 1:1 on screen.
- Full Screen Mode enlarges the primary view
  ([ADR 0014](archive/0014-fullscreen-mode-is-per-launcher-content-scaling.md)), so it shrinks all panels of
  that instance. Full Screen Mode therefore changes the visor layout, which ADR 0014 excluded. Title
  bars keep their regular size, as ADR 0019 requires.
- Nothing visible changes today. Every current application has one view, so its one panel is
  scaled by the same rule as today.

### A connector stage draws the connectors

- A connector joins two views of the same instance. Connectors into other instances are a follow-up.
- A connector is declared on its source view. Its source end names an optional port of that view.
  Its target end names the target view by its `ViewId`, and optionally one of that view's ports.
- A port belongs to a view. Its name is unique within that view, and its rect is in the view's own
  coordinates.
- A connector stays hidden until both of its ends resolve. An end does not resolve while its view
  does not exist or has no placement, or while the named port is not declared yet.
- A **connector stage** wraps one subtree of stages. It places its one child unchanged and draws the
  connector routes of all views below it.
- For now, one connector stage sits above every visor stage. It sees every panel of the visor, so
  that a route also avoids the views of other instances on the same arc.
- The connector stage receives the panel structure and the remembered views from the visor stage.
  It computes the route topology from the panel structure and sends the visor stage the connector
  lanes that each stack gap and each panel margin needs.
- The route topology changes when connectors, views or panels are added or removed, when a panel
  index changes, and when a nested instance's group moves. It does not change while panels slide, so
  the connector lanes stay the same then.
- While anything below it moves, the connector stage recomputes the route geometry every tick from
  the current positions that the desktop core composes.
- The connector stage draws the routes into its own scene location, above the views. A route fades
  with the views it connects. Connectors that leave a remembered view are emphasized.
- The connector stage is its own implementation of the `Stage` trait and not a `LayoutStage`. It
  draws from current positions instead of animating towards targets.
- The connector stage sees the placements of all panels below it, so it has what it needs to route
  a connector across a panel edge on the arc. The shape of that crossing is still open.

### An instance may start nested instances

- An application declares as a fixed property whether it starts nested instances. The property sits
  next to its draw target. The desktop refuses a start request from an application that does not
  declare it. This check lives in [security.rs](../../desktop/src/security.rs), because it limits
  what an instance may make the desktop do, and an instance may later run out of process.
- The parent instance asks for a nested instance through its `InstanceContext`. This API and the
  property live in `massive-applications`. The registry of applications, the start of the instance
  and the checks stay with the desktop.
- A nested instance must be an application that the desktop's registry knows. It inherits its parent
  instance's MCP root ([ADR 0036](archive/0036-an-mcp-root-exposes-a-project-subtree-to-agents.md)), so it
  can reach no more than its parent can.
- In the target tree, a nested instance hangs under the launcher of its parent instance. Its parent
  instance is an attribute, not an edge. So the parent instance is not on the nested instance's key
  path, and Cmd+W, Cmd+T and Cmd+Esc act on the nested instance itself. Its own role as a primary or
  an assistant instance decides which Full Screen Mode applies to it.
- The desktop knows every nested instance. Its views are ordinary targets for the focus, for hit
  testing and for MCP. `desktop_describe` lists a nested instance under its parent instance.
- A nested instance does not know that it is nested. It creates its views, declares their layout
  metadata and draws straight to the renderer, like every other instance.
- A nested instance's panels join its parent instance's visor as one group. The parent instance
  names after which of its own panels that group goes. It names the place in the start request and
  may change it later. Without a place, the group follows the parent's group.
- The parent instance owns the lifetime of its nested instances. It can stop one. When the parent
  instance ends, the desktop ends its nested instances. When a nested instance ends, its parent
  instance receives an event.
- The shell runs an application that starts nested instances. It answers every start request with an
  error, so such an application still runs on its own, and it decides how to do without its nested
  instances.

### The work proceeds in steps

1. A stage tree is introduced, with `SyncChildren`, as an exact mirror of the target tree. The
   layout runs over it, and nothing visible changes. Today's system tests guard this step.
2. `LayoutStage<L>`, the placed node, and the matrix and visor layouters replace the presenters'
   layout. A temporary shim derives their inputs from `LayoutContext`.
3. The visor stage's children become panels, and views with layout metadata arrive.
4. The shim is removed, once every input has its source on a stage's two sides.
5. The connector stage follows.
6. Nested instances come last.

## Considered options

- **An instance per project or per launcher.** Every level would be isolated, could crash on its own
  and could later run out of process. Every frame would pass through one more message hop per
  nesting level. Delivering a key from the top down would need a request and a reply per level.
  Placements could reach the level above a frame late. The camera, the focus path and hit testing
  would become protocols between instances. `MoveSlot`, `Group` and `Ungroup` would move running
  instances from one parent to another.
- **Stages that are designed to become instances later.** The plain data at the boundary covers
  most of this option anyway.
- **A window stage in the shell, with the desktop drawing into views.** A desktop could then run
  inside another desktop. It is the option that ADR 0039 rejected, and a camera inside another
  camera would need a design of its own.
- **One tree with exceptions for each role.** Panels, connector stages and nested instances would
  become special cases in the focus, layout and hit-testing code, and the special cases would grow
  with every new arrangement.
- **Three trees for ownership, focus and layout.** Focus and ownership differ only for nested
  instances, which an attribute covers. A third tree would add one more structure to keep
  consistent.
- **Dual writes to both trees.** Each of the three places that change the target tree would also
  carry the rules of the stage tree, and nothing structural would stop one tree from missing a
  write.
- **Rebuilding every stage's child list after every transaction.** It needs no marks, and the trees
  are small. It recomputes child lists that did not change on every transaction.
- **A stage per instance that places its panels on its part of the arc.** Arc parameters would go
  down and panel widths would come up. A nested instance's group placed between its parent's panels
  would split the parent's part of the arc, and scaling an instance node would distort the arc's
  radius in its coordinates.
- **The parent stage animates its children's placements.** The visor stage would hold the scene
  locations of its panels and views, which crosses ownership for views and for stages below.
- **Building the whole stage tree first and switching over at once.** It is clean from the start,
  but nothing between the first and the last change could ship, and today's tests would help only
  after the switch.
- **Connectors into other instances now.** A connector end could name the other instance's primary
  view with an optional port, views could get names, or the desktop could tell a parent the view ids
  of its nested instances. Nothing can use any of these before nested instances exist, and the last
  one brings back a round trip through the parent.
- **An unresolved connector end drawn in the view's margin.** The end would jump to its port once
  the port is declared.
- **Only the desktop may start instances.** No application needs nested instances today. The user
  chose to make them available to applications.
- **A nested instance hidden behind its parent.** The parent instance would receive the nested
  instance's events and pass them on. Every level would add a request and a reply per key. The
  desktop could not enforce the nested instance's MCP root, and it would draw the content of an
  instance it does not know.
- **A nested instance under its parent instance in the target tree.** The parent instance's level
  would see every key first (ADR 0020) and would take Cmd+W and Cmd+T for itself.
- **The parent instance forwards the nested instance's scene content.** Every nesting level would
  add a hop to every frame. The [out-of-process plan](../out-of-process-applications-plan.md) sets a
  performance bar that this would endanger for a terminal under heavy output.
- **The parent instance places the nested instance's views in its own panels.** The parent could
  stack its own views and the nested ones in one panel. Each nested view would need a round trip
  through the parent before it could appear, and the nested instance would be the one instance that
  does not lay out its own panels.
- **The layout stage in the instance routes the connectors**, as ADR 0045 planned. It cannot route
  across a panel edge on the arc, because it does not know the arc.
- **A specific stage per kind, with a pure layouter inside.** This was the first version of this
  decision. Each kind would write the protocol, the skipping and the animation again, and a new
  arrangement could not reuse them.
- **One layout engine for the whole tree.** Stages would only describe their nodes, and one solver
  would place everything. An arc, the sliding along the center line and connector routing do not fit
  a generic box layout, and ADR 0022 moved away from one central algorithm.
- **Connector routes in the layouters' output.** Every layouter's output would need routes that
  can be animated, and a connector could never leave its stage.
- **One connector space owned by the desktop core.** The desktop core would then know about
  connectors, which the stages exist to keep from it.
- **Connector routes bound to scene locations.** The renderer would evaluate every route each frame,
  and no stage would work per frame. One corner of a route takes its x from one location and its y
  from another, for example from a connector lane in one panel's margin and from a port in another
  panel's view. Every point would need an anchor per axis, and the crossing of the arc must be
  settled first. This stays a possible later optimization.
- **One connector stage at the root, or above every matrix stage.** Connectors would reach across
  launchers. Which views an instance may connect to is then a question of trust that is not
  settled.
- **Any instance may start nested instances.** Without a declaration, any running application could
  start other applications on the desktop, including an application that acts on input from a remote
  machine. That widens the trust model beyond what [security.md](../security.md) covers.
- **The hosted application, or the configuration, picks the stages above it.** If the application
  picked them, it would take decisions that belong to the desktop policy
  ([ADR 0029](archive/0029-exceptional-desktop-behaviors-are-decided-by-the-desktop-policy.md)). If the
  configuration picked them, it would bring back the mode that
  [ADR 0044](archive/0044-launchers-present-only-as-visors.md) removed, without a second stage that serves
  someone.
- **Layout and presentation as two stages in series.** One presentation stage would animate every
  level. The placement it receives would have to carry transitions and visibility, and the layout
  focus would pass through two stages per level.
- **A closed set of stages.** New arrangements, such as rows in a visor, are expected, and the
  traits make every stage follow the same constraint from the start.
- **The scale depends on the panel count.** An instance with one panel would be scaled to fit, and
  an instance with several panels would be shown 1:1 while expanded. An instance would change its
  scale the moment it opens a second panel.
- **Every expanded panel at 1:1.** A full-screen terminal would cover its neighboring slots at the
  Slot, Row and Project zoom levels, which ADR 0045 rejected.
- **The shell refuses an application that starts nested instances.** It would fail fast, but such an
  application could never run on its own.
- **The shell runs nested instances, one window each.** The application would behave the same under
  both hosts. The shell would need a registry, starts and event routing across windows, the check of
  the declared property would exist twice, and the MCP root has no meaning without a desktop socket.
- **A separate channel for layout metadata and requests to the parent.** Scene changes would stay
  apart from everything else. Layout metadata must take effect in the same frame as the scene
  changes that it positions, so a second channel would need sequence numbers or fences to restore
  the order of the one channel.

## Consequences

- The presenters' layout of ADR 0022 becomes the layout of stages. Every value that a presenter now
  reads through `LayoutContext` needs a source on one of the stage's two sides. Finding these
  sources is most of the work of the matrix stage.
- ADR 0045 changes in several sections. Its two kinds of applications are gone. Its two ways of
  scaling become the one instance scale. Its layout stage in the instance becomes the visor stage
  and the connector stage in the desktop, and connector and port metadata now cross into the
  desktop. The center visor view moves from the desktop core into the visor stage.
- ADR 0045's open questions get answers. Full Screen Mode shrinks every panel of its instance. A
  view without indices stays hidden, except the primary view, which defaults to panel 0, stack 0.
  The connector stage holds what a connector needs to cross the arc, but the shape of the crossing
  stays open.
- ADR 0014 no longer holds where it says that Full Screen Mode never changes the visor layout.
- The visor panels documents [03](../visor-panels/03-panels-on-the-visor.md),
  [05](../visor-panels/05-layout-stage.md), [06](../visor-panels/06-visor-layouter.md),
  [07](../visor-panels/07-connector-routing.md) and
  [08](../visor-panels/08-animation-and-drawing.md) record the details.
- Connector routing runs in the desktop's task. A large connector graph could therefore slow the
  desktop and not only one instance. The route topology runs only when the structure changes, and
  the route geometry runs every tick while something moves. Both costs are measured before they are
  optimized.
- The renderer needs the connector shapes of visor panels document
  [02](../visor-panels/02-connector-shapes.md). Two renderer questions are open. It is not yet
  checked what draws the connectors above the views, and an arrowhead may need a new shape.
- Both security documents change in the change that introduces nested instances. They gain the
  declared property, the check that only a declaring application may start nested instances, and the
  rule that a nested instance inherits its parent's MCP root.
- `desktop_describe` lists nested instances under their parent instance.
- The glossary gains desktop core, target tree, stage tree, stage, layout stage, layouter, matrix
  stage, visor stage, connector stage, placed node, layout metadata, panel structure, stack gap,
  panel margin, connector lane, connector route, route topology, route geometry, parent instance,
  nested instance and instance scale, in the changes that introduce them. Layout focus is redefined
  to name a view. Stage is never used for the lifetime stage
  ([ADR 0040](0040-a-lifetime-stage-before-the-renderer-decides-when-scene-objects-end.md)), which
  keeps its full name. The glossary avoids "gap" and "lane" for other meanings, so the qualified
  terms are used. The terms multi-instance application and multi-panel application of ADR 0045 are
  not introduced.
- The channel from an instance to the desktop stays as it is, with scene changes, view changes and
  requests interleaved in one ordered submission. A later change could structure each submission
  into a scene part and a part for the desktop, and send requests as messages of their own on the
  same channel. That would separate the types without losing the order.

## Follow-ups

- Connectors into other instances. `ViewId` is a random UUID
  ([lib.rs](../../applications/src/lib.rs)), so a view id is unique across the whole desktop, and a
  connector end could name a view of another instance without any other qualifier. What is open is
  which view ids an instance may name, and how it learns them. The likely first shape is the primary
  view of a nested instance, with an optional port.

## Open questions

- The message types of the `Stage` trait.
- Whether a stage will need a say about key presses.
- How a visor gets rows, so that a parent instance can place nested instances below its own panels.
  Panels can have arbitrary heights, which this has to take into account.
- Whether connector stages may sit higher in the tree, so that connectors join views of different
  launchers, and which views an instance may then connect to.
- How a connector crosses from one panel to the next on the arc.
- Whether connector routes are later bound to scene locations, so that the renderer moves them and
  no stage works per frame.
- Whether a nested instance may itself start nested instances.
- How a connector to a view that has closed is handled. It is likely dropped silently.
- Whether mt can register a layouter.
- Whether `desktop_describe` lists an instance's panels and views.
