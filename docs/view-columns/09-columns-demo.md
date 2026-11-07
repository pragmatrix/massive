# 09 Columns demo

Status: **planned** (2026-11-07). Part of [view columns](README.md). Depends on all other
documents.

## Goal

A scripted demo application shows view columns working end to end in the real desktop. It is the
check that the parts fit together. It is not a product.

## Decisions

- **It lives in mt as `columns-demo`.** An instance with several views runs only in the desktop
  ([03](03-multi-view-instances.md)). massive's examples run in the shell, which shows one view per
  instance.
- **A script fills it.** The script creates views over time, connects them, and closes some of them.
  No outside input drives it.
- **It is verified with `run-mt`.**

## What the script covers

- Views of different heights in several columns.
- Connectors to the next column, connectors that skip a column, backward connectors, and a connector
  within one column.
- Ports that move while the view's content changes.
- Enough views that the content extent exceeds the panel and the fit scale drops below 1.
- Moving the focus up, down, left, and right, so that columns realign and the camera follows.
- Closing a selected view.

## Done when

- Text in the focused view is pixel-perfect at the Focus zoom level ([01](01-pixel-perfect-focus.md)).
- No connector crosses a view, apart from the runs between its ports and their views' edges
  ([07](07-connector-routing.md)).
- Views and connectors move together without detaching ([08](08-presentation-layer.md)).

## Open questions

- Whether the demo is useful enough to keep after the feature is done, or is removed.
