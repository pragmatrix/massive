# 03 Multi-view instances

Status: **decisions taken, design to be fleshed out** (2026-11-07). Part of
[view columns](README.md). Depends on [01](01-pixel-perfect-focus.md).

## Goal

The desktop hosts several views per instance, each at a position that the instance chooses. The
desktop scales the instance's content to fit its panel. It knows nothing about columns, connectors,
ports, or the selection.

## Current situation

- An instance can present exactly one view. `present_view` rejects every view that is not the
  primary view, and a second primary view
  ([instance_presenter.rs:299](../../desktop/src/instance_presenter.rs#L299)).
- The desktop positions the one view itself. A view is a part of its instance
  (`InstancePart::View`), and the instance presenter lays it out
  ([ADR 0022](../adr/0022-presenters-own-the-layout-of-their-targets.md)).
- A view states its size when it is created (`ViewCreationInfo`). `ViewChange::Resize` exists, but
  the desktop handles it with `todo!()`
  ([command_dispatch.rs:892](../../desktop/src/desktop_system/command_dispatch.rs#L892)).
- The instance title bar shows the title of the primary view
  ([ADR 0019](../adr/0019-instance-title-bar-is-part-of-the-instance-extent.md)).
- Every slot in a matrix has the same size, the slot extent
  ([ADR 0023](../adr/0023-matrix-slots-share-one-slot-extent-and-center-their-content.md)). So an
  instance cannot grow beyond its panel without growing every slot of its project.

## Decisions

- **The one-view limit is lifted.** An instance can present any number of views.
- **The instance sends a position per view.** It is the view's top-left corner in the instance's
  content coordinates. A new view change carries only the position, because a layouter never
  resizes a view.
- **The instance sends its content extent.** A new instance change carries the size of everything
  the instance has placed.
- **The desktop fits the content extent into the panel.** It uses the same fit as Full Screen Mode,
  `fullscreen_scale` ([fullscreen.rs:30](../../desktop/src/desktop_system/fullscreen.rs#L30)), so the
  fit is computed in one place. The scale never goes above 1. Content smaller than the panel stays at
  full size, centered.
- **The scale animates.** When the content extent changes, the desktop eases to the new scale with
  its structural animation.
- **The desktop applies positions as they arrive.** It does not ease view positions itself, because
  the instance animates the drawing of its views ([08](08-presentation-layer.md)).
- **Hit testing and navigation use the positions.** A click hits a view at the position the
  instance sent. The arrow keys move the keyboard focus between views by their positions, as they
  move it between slots.
- **The title bar shows the focused view's title.**
- **The shell is unchanged.** It still shows one view per instance, so an instance with several
  views runs only in the desktop
  ([ADR 0039](../adr/0039-the-shell-hosts-one-instance-and-the-desktop-is-that-instance.md)).

## Glossary changes

- Define **panel** and **content extent**.
- Widen **Selection** to views. The desktop's entry then describes a state that the layout stage
  keeps, not the desktop ([05](05-layout-stage.md)).
- Update **Instance title bar**, which today names the primary view.

## Open questions

- The names of the new view change and instance change.
- Which title the title bar shows while the keyboard focus is outside the instance. The last
  focused view's title is the likely answer.
- What happens to a view that is created but has no position yet. It could stay hidden until its
  first position arrives.
- Whether `ViewRole` still means anything for an instance with several views.
- Whether the MCP tool `desktop_describe` should list an instance's views. If it does, both security
  documents ([massive](../security.md), [mt](../../../docs/security.md)) are updated in the same
  change.
- How placement visibility ([ADR 0001](../adr/0001-placement-visibility-drives-interaction-and-fade.md))
  applies to several views of one instance.
