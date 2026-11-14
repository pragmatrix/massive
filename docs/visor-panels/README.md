# Visor panels

Status: **draft, decisions taken, not approved** (2026-11-07, revised 2026-11-14). The decisions
were taken in two design sessions. The first one planned view columns inside one instance. The
second one unified view columns with the visor and is recorded in
[ADR 0045](../adr/0045-a-visor-is-an-arc-of-panels-laid-out-by-a-stage.md). Each document in this
directory covers one part of the work and lists its own decisions and open questions. The documents
are meant to be fleshed out further before any of them is implemented.

## Goal

An instance should be able to present many views at once, arranged in panels on its launcher's
visor arc and joined by connectors. The arrangement is a view to look at, not a graph editor. The
user never drags views or draws connectors. The application declares which panel each view belongs
to and how the views connect, and a layout stage decides where they go.

The arrangement is not tied to one kind of application. Nothing in this plan depends on what the
views show.

## Terms

These terms are used across all documents here. They go into [GLOSSARY.md](../../GLOSSARY.md) in
the step that introduces them.

- **Panel**: one unit of a visor. A panel holds a vertical stack of one or more views. A launcher's
  visor places its panels side by side on one arc, with no gap between them. This replaces the term
  "view column" of the first session. It also replaces the earlier meaning of "panel", which was
  the rect that holds all of an instance's content.
- **Multi-instance application**: an application whose instances each present one panel. Every
  application is multi-instance today.
- **Multi-panel application**: an application whose launcher runs exactly one instance, and that
  instance presents any number of panels. An application is either multi-instance or multi-panel,
  and it states which as a fixed property.
- **Panel index** and **stack index**: the two numbers a multi-panel application gives each view.
  The panel index says which panel the view belongs to. The stack index says where in the panel's
  stack the view sits.
- **Primary view**: the view with the role `ViewRole::Primary`. Every instance has exactly one. A
  collapsed visor of a multi-panel launcher shows only its primary view.
- **Remembered view**: the one view per panel that the layout stage remembers. It is the view of the
  panel that most recently held the layout focus.
- **Center visor view**: the one view per launcher that the desktop remembers. Re-entering the
  launcher focuses it, and the arc centers on its panel. It generalizes the center visor instance.
- **Center line**: the horizontal line at y = 0 that all of a multi-panel instance's panels are laid
  out against. The desktop places it at the launcher's vertical center.
- **Connector**: a one-way line from one view to another view of the same instance. It can cross
  panel edges. It has no kind and no other properties for now.
- **Port**: a named rect inside a view, in the view's own coordinates. A rect of zero size is a
  point. A connector end names a view and optionally one of its ports. A connector starts at a port
  and leaves it to the right, and ends at a port, arriving from the left. Without a port, the end
  sits in the view's margin.
- **Layouter**: a pure function that turns the views' sizes and indices, their connectors, the
  remembered views, and the layout focus into panel sizes and offsets, view positions, and connector
  routes.
- **Presentation layer**: the part of the layout stage that sends the layouter's results to the
  desktop, draws the connectors, and animates views and connectors together.
- **Focus companion**: a view that the camera frames together with another view while that other
  view has the keyboard focus.
- **Lane**: the room one connector takes up where it passes through a gap or a margin.

## How the work divides

The work spans three places. The application only declares. The layout stage, which sits between
the application and the desktop, decides and animates. The desktop hosts the panels on the arc.

```
application ──views, indices,──▶ layout stage ──panels, view positions──▶ desktop
            connectors, ports    (layouter +
            ◀──────events──────── presentation ◀──layout focus, events───
                                  layer)
```

- **The application** creates views. It gives each view a panel index and a stack index, and it
  declares connectors and ports. It never positions a view, and it knows nothing about the layout
  stage besides which stage it uses.
- **The layout stage** sits between the instance's submissions and the events it receives
  ([05](05-layout-stage.md)). Its layouter sizes the panels, stacks their views, and slides each
  panel vertically ([06](06-panel-layouter.md)). It routes the connectors around the views
  ([07](07-connector-routing.md)). Its presentation layer draws and animates the result
  ([08](08-presentation-layer.md)).
- **The desktop** places the panels on the visor arc, scales them, and moves the keyboard focus
  between views ([03](03-multi-panel-instances.md)). Its camera frames the focused view with its
  focus companion ([04](04-focus-companions.md)). It knows nothing about connectors, ports, or the
  remembered views.

Because the layout stage runs on the instance's side, it also works for applications that run out
of process ([out-of-process plan](../out-of-process-applications-plan.md)). Only panels, view
positions, and scene changes cross the boundary towards the desktop, and only the layout focus is
added in the other direction.

## Documents

| Document | Part | Depends on |
|---|---|---|
| [02 Connector shapes](02-connector-shapes.md) | Renderer: rects, arcs, biarcs, endpoint circles as SDFs | – |
| [03 Multi-panel instances](03-multi-panel-instances.md) | Desktop: application kinds, panels on the arc, scale, navigation | – |
| [04 Focus companions](04-focus-companions.md) | Desktop: what the camera frames at Focus | 03 |
| [05 Layout stage](05-layout-stage.md) | Applications: metadata API, `Layouter` trait, the stage | 03 |
| [06 Panel layouter](06-panel-layouter.md) | Panel sizes, stacks, sliding, remembered views | 05 |
| [07 Connector routing](07-connector-routing.md) | Routes around views, gaps, margins, lanes | 02, 06 |
| [08 Presentation layer](08-presentation-layer.md) | Drawing and animating views and connectors | 02, 05, 07 |
| [09 Panels demo](09-panels-demo.md) | A scripted multi-panel demo application in mt | all |

[biarc-vs-cubic.svg](biarc-vs-cubic.svg) compares the connector curves that were considered. It is
the evidence for the decision in [02](02-connector-shapes.md).

`massive` is its own repository. Its changes land through massive pull requests, and mt then moves
the submodule pointer. The demo ([09](09-panels-demo.md)) is the only part that lives in mt.

## Open questions

These questions concern the plan as a whole. Each document lists its own as well.

- How a connector crosses from one panel to the next on the arc. It may need to bend smoothly along
  the arc. The answer can change the arc's own shape, for example flat panels hinged at their
  edges, or panels on a circle as today.
- How Full Screen Mode works in a multi-panel launcher.
- Whether `desktop_describe` lists an instance's panels and views.
- What happens to a view whose application has not yet given it a panel index and a stack index.
- How views appear and disappear.

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
