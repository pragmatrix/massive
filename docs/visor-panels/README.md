# Visor panels

Status: **draft, decisions taken, not approved** (2026-11-07, revised 2026-11-14). The decisions
were taken in three design sessions. The first one planned view columns inside one instance. The
second one unified view columns with the visor and is recorded in
[ADR 0045](../adr/0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md). The third one moved the
layout from the instance into the desktop and is recorded in
[ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md). Each document in this directory
covers one part of the work and lists its own decisions and open questions. The documents are meant
to be fleshed out further before any of them is implemented.

## Goal

An instance should be able to present many views at once, arranged in panels on its launcher's
visor arc and joined by connectors. A connector may also join views of different instances of one
launcher. The arrangement is a view to look at, not a graph editor. The user never drags views or
draws connectors. The application declares which panel each view belongs to and how the views
connect, and the desktop decides where they go.

The arrangement is not tied to one kind of application. Nothing in this plan depends on what the
views show.

## Terms

These terms are used across all documents here. They go into [GLOSSARY.md](../../GLOSSARY.md) in
the step that introduces them.

- **Panel**: one unit of a visor. A panel holds a vertical stack of one or more views of one
  instance. A launcher's visor places the panels of all its instances side by side on one arc, with
  no gap between them. This replaces the term "view column" of the first session. It also replaces
  the earlier meaning of "panel", which was the rect that holds all of an instance's content.
- **Layout metadata**: what a view declares about its place. It consists of a panel index, a stack
  index, the view's ports, and its outgoing connectors.
- **Panel index** and **stack index**: the two numbers in a view's layout metadata. The panel index
  says which of its instance's panels the view belongs to. The stack index says where in the panel's
  stack the view sits. Both count within the view's instance.
- **Primary view**: the view with the role `ViewRole::Primary`. Every instance has exactly one. It
  sits in panel 0, stack 0, unless the instance declares otherwise.
- **Instance scale**: the largest scale, at most 1, at which an instance's primary view fits the
  slot. All panels of the instance are shown at it.
- **Remembered view**: the one view per panel that the visor stage remembers. It is the view of the
  panel that most recently held the layout focus.
- **Center visor view**: the one view per launcher that the visor stage remembers. Re-entering the
  launcher focuses it, and the arc centers on its panel. It generalizes the center visor instance.
- **Center line**: the horizontal line at y = 0 that all panels of a visor are laid out against. It
  lies at the launcher's vertical center.
- **Connector**: a one-way line from one view to another view. Both views must be below the same
  connector stage. A connector can cross panel edges. It has no kind and no other properties for
  now.
- **Port**: a named rect inside a view, in the view's own coordinates. A rect of zero size is a
  point. A connector end names an instance, one of its views, and optionally one of the view's
  ports. A connector starts at a port and leaves it to the right, and ends at a port, arriving from
  the left. Without a port, the end sits in the view's margin.
- **Layout stage**: the part of the desktop that lays out the children of one level, between that
  level's parent side and its child side. One generic `LayoutStage<L: Layouter>` implements every
  layout stage ([05](05-layout-stage.md)).
- **Layouter**: the pure function that a layout stage calls. It takes the stage's state, inputs and
  events, and it returns the stage's new state and its output.
- **Visor stage**: the layout stage of a launcher, with the visor layouter
  ([06](06-visor-layouter.md)).
- **Connector stage**: the stage above a visor stage that routes and draws the connectors of the
  views below it ([07](07-connector-routing.md)).
- **Connector route**: the path drawn for one connector. Its **route topology** says which gap or
  which way around a stack it takes in every panel it passes, and which lane it uses. Its **route
  geometry** is its exact lines and arcs.
- **Lane**: the room one connector takes up where it passes through a gap or a margin.
- **Focus companion**: a view that the camera frames together with another view while that other
  view has the keyboard focus.

## How the work divides

The work spans the application and two stages in the desktop. The application only declares. The
visor stage places the views. The connector stage above it routes and draws the connectors.

```
                       desktop core (camera, focus, hit testing, aggregate)
                          │ topology, layout focus, current placements
                          ▼
                    connector stage ──draws connector routes
                     │ lanes ▲ panel structure
                     ▼       │
                     visor stage ──places panels and views on the arc
                          ▲
application ──views with layout metadata (indices, ports, connectors)
```

- **The application** creates views and gives each view its layout metadata
  ([05](05-layout-stage.md)). It never positions a view. An application whose instances have one
  view each declares nothing and behaves as today.
- **The visor stage** joins the panels of all instances of its launcher on the arc, stacks each
  panel's views, slides each panel along the center line, and scales each instance by its instance
  scale ([03](03-panels-on-the-visor.md), [06](06-visor-layouter.md)).
- **The connector stage** computes the route topology from the connectors and the panel structure,
  and it tells the visor stage how many lanes each gap and margin needs. While anything moves, it
  recomputes the route geometry every tick from the current placements
  ([07](07-connector-routing.md)).
- **The generic layout stage** animates the placements, and the connector stage draws from the
  current ones, so views and connectors move together ([08](08-animation-and-drawing.md)).
- **The desktop core** moves the keyboard focus between views by position. Its camera frames the
  focused view with its focus companion ([04](04-focus-companions.md)).

The layout metadata is plain data that travels with the instance's view changes. So the plan also
works for applications that run out of process
([out-of-process plan](../out-of-process-applications-plan.md)). The layout focus no longer crosses
into the instance.

## Documents

| Document | Part | Depends on |
|---|---|---|
| [02 Connector shapes](02-connector-shapes.md) | Renderer: rects, arcs, biarcs, endpoint circles as SDFs | – |
| [03 Panels on the visor](03-panels-on-the-visor.md) | Desktop: instances and panels on the arc, scale, navigation | – |
| [04 Focus companions](04-focus-companions.md) | Desktop: what the camera frames at Focus | 03 |
| [05 Layout stage](05-layout-stage.md) | Layout metadata, the generic layout stage, the `Stage` and `Layouter` traits | 03 |
| [06 Visor layouter](06-visor-layouter.md) | Panel sizes, stacks, sliding, remembered views | 05 |
| [07 Connector routing](07-connector-routing.md) | The connector stage: routes around views, gaps, margins, lanes | 02, 06 |
| [08 Animation and drawing](08-animation-and-drawing.md) | How views and connectors move and are drawn | 02, 05, 07 |
| [09 Panels demo](09-panels-demo.md) | A scripted demo application with several views in mt | all |

[biarc-vs-cubic.svg](biarc-vs-cubic.svg) compares the connector curves that were considered. It is
the evidence for the decision in [02](02-connector-shapes.md).

`massive` is its own repository. Its changes land through massive pull requests, and mt then moves
the submodule pointer. The demo ([09](09-panels-demo.md)) is the only part that lives in mt.

## Open questions

These questions concern the plan as a whole. Each document lists its own as well.

- How a connector crosses from one panel to the next on the arc. It may need to bend smoothly along
  the arc. The answer can change the arc's own shape, for example flat panels hinged at their
  edges, or panels on a circle as today.
- Whether `desktop_describe` lists an instance's panels and views.
- How views appear and disappear.
- How a visor gets rows, so that a parent instance can place its nested instances below its own
  panels ([ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md)).

## Follow-ups, not part of this plan

- Applications that fill the panels.
- Views that change their own size. `ViewChange::Resize` is still a `todo!()` in the desktop.
- A scene that animates values on its own, so that animated transforms need not be sent every
  frame.
- Connector kinds and labels.
- Other sliding strategies, for example aligning panels along their connectors, and whether users
  can choose the strategy.
- Panels as a tree, where the remembered view decides what the next panel shows.
- A title bar per view.
- More than one focus companion per view.
- Nested instances ([ADR 0046](../adr/0046-projects-and-launchers-are-layout-stages.md)), which
  bring their own panels into their parent instance's visor.
