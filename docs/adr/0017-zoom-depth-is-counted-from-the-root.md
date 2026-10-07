# Zoom depth is counted from the root

## Status: superseded by [ADR 0018](./0018-zoom-is-a-level-relative-to-the-focused-target.md)

Supersedes [ADR 0016](./0016-camera-zoom-is-relative-to-keyboard-focus.md) and
the Focus Depth sequence of
[ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md).

## Problem

ADR 0016 counted camera zoom as zoom-out steps outward from the keyboard-focused
target's innermost framing, preserved across focus changes and clamped to the
new target's path. Targets sit at different nesting depths, so the same step
count lands on a different ancestor once focus moves to a target nested at
another level: the camera jumps while the user only wanted to navigate inside
the frame they were looking at.

## Decision

Zoom is a **zoom depth** counted inward from the root, along the focused
target's **zoom chain**. Each entry of the chain is a **zoom level** — `Project`,
`Row`, `Slot` or `Instance` — within a project, and its position in the chain is
its **zoom index**:

```text
Project(root), Row, Slot, Row, Slot, …, Instance
```

Zoom index 0 frames the root project. Each nested project contributes `Row,
Slot`; its own `Project` level merges into the hosting parent's `Slot` level,
because both frame the same presented rect. The virtual `Desktop` target is not
part of the chain. A launcher target's chain ends at its `Slot`; an instance
adds `Instance`.

```rust
enum ZoomDepth {
    Depth(usize),
    Innermost,
}
```

- `Depth(n)` is stored unclamped. It resolves to zoom index `min(n, len - 1)`
  on the focused target's chain, so moving from a deep target to a shallow one
  and back restores the same zoom level. Focus changes do not modify the depth.
- `Innermost` frames the end of whatever chain the focused target has. It is
  sticky: a deeper or newly started target is framed fully in as well.
- Zoom commands normalize to the resolved zoom index first, so they always
  change the zoom level: `ZoomOut` from `Innermost` yields `Depth(len - 2)`; `ZoomIn` that
  reaches the end of the chain yields `Innermost`; `ZoomOut` at depth 0 and
  `ZoomIn` at `Innermost` are no-ops.
- A click, `Zoom::Reset`, and starting or presenting an instance set
  `Innermost`.
- "Fully zoomed in" — the condition `Cmd+Enter` and the Full Screen Mode toggle
  test — is the resolved zoom index being the end of the focused target's chain, not
  the `Innermost` variant: an unclamped `Depth(n)` on a short chain is fully in
  too.
- An outward step never decreases camera distance, even when its framed bounds
  are narrower (unchanged from ADR 0016).

### Navigation

At a given depth the framed ancestor stays fixed while focus moves inside it,
so arrows navigate at the framed level's granularity: the focused target maps
to the slot of the framed project that contains it, and the result resolves
back to a concrete target through each project's focus slot (one remembered
immediate slot on the most recently focused path, continued through nested
projects) and the launcher's instance anchor.

- Navigating onto a project slot focuses the concrete target resolved through
  the nested project's focus slot (or its first launcher, depth first), and
  the nested `Project` target only when the project holds no launcher.
- Navigating past the framed ancestor's matrix edge overflows into the parent
  slot's sibling sequence (column affinity unchanged); the zoom level is recomputed
  from the new target's chain at the same depth, so the camera follows to the
  sibling ancestry.
- A `Project` target is a valid navigation origin.

### Cameras and scale

`Project` and `Slot` levels fit the presented (scaled) rect of a nested project
directly. Deeper levels compensate for the target placement's composed presentation scale
so focused content returns to its local pixel scale; no per-level scale is
reconstructed.

`project_of_target` resolves `Desktop` *down* to the root project — the one
documented exception, logged, because a hit-test miss produces a `Desktop`
target — so every target has a chain that starts at the root.

### Indicator

The zoom level indicator shows the zoom level and the nesting depth of the
framed project, root = 0: `Project 0`, `Row 1`, `Instance 2`. The framed
project, not the focused target's project, is shown — they differ when zoomed
out. The badge is sized for the widest level label plus two digits so it does
not resize between depths.

## Consequences

- `ZoomOutSteps` and `CommitZoomOutSteps` are replaced by `ZoomDepth` and
  `CommitZoomDepth`, `FramingLevel` by `ZoomLevel`; the clamp on focus change
  is removed.
- Navigation inside a zoomed-out level no longer moves the camera unless focus
  leaves the framed ancestor.
- Starting an instance forces `Innermost`, which matches the previous behaviour
  of resetting the zoom-out count to zero.

## Considered options

- **Zoom-out steps relative to the focused target (ADR 0016).** Rejected: the
  framed ancestor depends on the target's nesting depth, so navigation across
  levels moves the camera.
- **A depth clamped on every focus change.** Rejected: lossy — a detour through
  a shallow target permanently zooms out the deep one.
- **An unclamped depth without an `Innermost` state.** Rejected: "fully in"
  would not survive a focus change to a deeper target, such as an instance
  started from a launcher.
