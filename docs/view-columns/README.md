# View columns

Status: **draft, decisions taken, not approved** (2026-11-07). The decisions were taken in a design
session. Each document in this directory covers one part of the work and lists its own decisions
and open questions. The documents are meant to be fleshed out further before any of them is
implemented.

## Goal

An instance should be able to present many views at once, arranged in columns and joined by
connectors. The arrangement is a view to look at, not a graph editor. The user never drags views
or draws connectors. The application declares what its views are and how they connect, and a
layout algorithm decides where they go.

The arrangement is not tied to one kind of application. Nothing in this plan depends on what the
views show.

## Terms

These terms are used across all documents here. They go into [GLOSSARY.md](../../GLOSSARY.md) in
the step that introduces them.

- **View column**: a vertical stack of views inside one instance. All view columns of an instance
  have the same width, which the instance sets. "Column" alone means a view column throughout this
  directory. The glossary needs the longer name wherever matrix columns could be meant.
- **Connector**: a one-way line from one view to another view of the same instance. It has no kind
  and no other properties for now.
- **Port**: a named rect inside a view, in the view's own coordinates. A rect of zero size is a
  point. A connector end names a view and optionally one of its ports. A connector starts at a port
  and leaves it to the right, and ends at a port, arriving from the left. Without a port, the end
  sits in the view's margin.
- **Selection**: the glossary's existing term, widened to views. Every view column has one selected
  view. The keyboard focus selects a view, and a column keeps its selected view when the focus
  moves on.
- **Panel**: the rect that holds an instance's content, below its title bar. Its size is the default
  panel size, or the window in Full Screen Mode. The glossary uses the word today without defining
  it.
- **Content extent**: the size of everything an instance has placed, in its own coordinates. The
  desktop scales the content extent down to fit the panel.
- **Layouter**: a pure function that turns the views' metadata and the keyboard focus into view
  positions, connector routes, the content extent, and focus companions.
- **Presentation layer**: the part on the instance's side that sends the layouter's results to the
  desktop, draws the connectors, and animates views and connectors together.
- **Focus companion**: a view that the camera frames together with another view while that other
  view has the keyboard focus.
- **Lane**: the room one connector takes up where it passes through a gap.

## How the work divides

The work spans three places. The application only declares. The layout stage, on the instance's
side, decides and animates. The desktop only hosts.

```
application ──views + metadata──▶ layout stage ──positions, extent, companions──▶ desktop
            ◀──────events──────── (layouter +   ◀──────── focus, other events ────
                                   presentation
                                   layer)
```

- **The application** creates views and declares their connectors and ports. It never positions a
  view.
- **The layout stage** sits between the instance's submissions and the events it receives
  ([05](05-layout-stage.md)). Its layouter places the views in columns ([06](06-column-layouter.md))
  and routes the connectors around them ([07](07-connector-routing.md)). Both follow the layered
  graph drawing framework, except for how columns are placed vertically. Its presentation layer
  draws and animates the result ([08](08-presentation-layer.md)).
- **The desktop** hosts several views per instance at the positions it is given
  ([03](03-multi-view-instances.md)). It scales the content to fit the panel, and its camera frames
  the focused view at 1:1 ([04](04-focus-companions.md)). It knows nothing about columns,
  connectors, ports, or the selection.

Because the layout stage runs on the instance's side, it also works for applications that run out
of process ([out-of-process plan](../out-of-process-applications-plan.md)). Only view positions and
scene changes cross the boundary, and scene changes already do.

## Documents

| Document | Part | Depends on |
|---|---|---|
| [01 Pixel-perfect focus](01-pixel-perfect-focus.md) | Experiment: the camera cancels the fit scale | – |
| [02 Connector shapes](02-connector-shapes.md) | Experiment and renderer: rects, arcs, biarcs, endpoint circles as SDFs | – |
| [03 Multi-view instances](03-multi-view-instances.md) | Desktop: several views per instance, the content extent | 01 |
| [04 Focus companions](04-focus-companions.md) | Desktop: what the camera frames at Focus | 03 |
| [05 Layout stage](05-layout-stage.md) | Applications: metadata API, `Layouter` trait, the stage | 03 |
| [06 Column layouter](06-column-layouter.md) | Columns, order, alignment, selection | 05 |
| [07 Connector routing](07-connector-routing.md) | Routes around views, gaps, lanes | 02, 06 |
| [08 Presentation layer](08-presentation-layer.md) | Drawing and animating views and connectors | 02, 05, 07 |
| [09 Columns demo](09-columns-demo.md) | A scripted demo application in mt | all |

[biarc-vs-cubic.svg](biarc-vs-cubic.svg) compares the connector curves that were considered. It is
the evidence for the decision in [02](02-connector-shapes.md).

## Sequencing

The two experiments come first, because the rest of the design rests on them. If text cannot stay
pixel-perfect under the fit scale ([01](01-pixel-perfect-focus.md)), the desktop half of the design
changes. If the arc SDF does not join cleanly ([02](02-connector-shapes.md)), the connector style
changes. Neither experiment depends on the other, so they can run in parallel.

After the experiments, the desktop work ([03](03-multi-view-instances.md),
[04](04-focus-companions.md)) and the renderer work ([02](02-connector-shapes.md)) can proceed in
parallel. The layout stage ([05](05-layout-stage.md)) needs the desktop's new view changes. The
column layouter and the router ([06](06-column-layouter.md), [07](07-connector-routing.md)) are pure
functions and can be built and tested before the stage exists.

`massive` is its own repository. Its changes land through massive pull requests, and mt then moves
the submodule pointer. The demo ([09](09-columns-demo.md)) is the only part that lives in mt.

## ADR candidates

- An instance places its own views, and the desktop fits the instance's content into its panel.
- A view can name a focus companion that the camera frames with it at the Focus zoom level. This
  relates to ADR 0016 and ADR 0018.
- Selection covers views as well as slots. This amends ADR 0025 and ADR 0030 for views.
- A layout stage between an instance's submissions and its events owns the layout of its views.

## Follow-ups, not part of this plan

- Applications that fill the columns.
- Views that change their own size. `ViewChange::Resize` is still a `todo!()` in the desktop.
- A scene that animates values on its own, so that animated transforms need not be sent every
  frame.
- Connector kinds and labels.
- Columns as a tree, where the selection decides what the next column shows.
- A title bar per view.
- More than one focus companion per view.
