# 09 Panels demo

Status: **planned** (2026-11-07, revised 2026-11-14). Part of [visor panels](README.md). Depends on
all other documents.

## Goal

A scripted demo application shows visor panels working end to end in the real desktop. It is the
check that the parts fit together. It is not a product.

## Decisions

- **It lives in mt as `panels-demo`.** It is a multi-panel application, and multi-panel applications
  run only in the desktop ([03](03-multi-panel-instances.md)). massive's examples run in the shell,
  which shows one view per instance.
- **A launcher starts it by name.** The launcher's `application` field names `panels-demo`
  ([03](03-multi-panel-instances.md)).
- **A script fills it.** The script creates views over time, gives them panel and stack indices,
  connects them, and closes some of them. No outside input drives it.
- **It is verified with `run-mt`.**

## What the script covers

- Views of different widths and heights in several panels, so that panels differ in width and
  height.
- A primary view larger than the slot, so that the collapsed visor fits it into the slot.
- Panels taller than the slot, which extend beyond it while the visor is expanded.
- Connectors to the next panel, connectors that skip a panel, connectors to a panel on the left,
  and a connector within one panel.
- Ports that move while the view's content changes.
- Moving the focus up, down, left, and right, so that panels slide, the arc recenters, and the
  camera follows.
- Leaving and re-entering the launcher, so that the visor collapses and expands.
- Closing a remembered view.

## Done when

- No connector crosses a view, apart from the runs between its ports and their views' edges
  ([07](07-connector-routing.md)).
- Views and connectors move together without detaching ([08](08-presentation-layer.md)).
- The focused view sits on the launcher's vertical center after every focus move
  ([06](06-panel-layouter.md)).

## Open questions

- Whether the demo is useful enough to keep after the feature is done, or is removed.
