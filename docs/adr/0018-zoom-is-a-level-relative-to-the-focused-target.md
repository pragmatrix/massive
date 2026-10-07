# Zoom is a level relative to the focused target

## Status: accepted

Supersedes [ADR 0017](./0017-zoom-depth-is-counted-from-the-root.md).

## Problem

ADR 0017 counts zoom as a depth from the root and lets arrow keys navigate at
the framed level's granularity: while zoomed out, the keyboard-focused target
stays a deep instance, but navigation acts on the slot of the framed ancestor
that contains it. The same arrow key therefore behaves differently depending on
the zoom depth, and the focus the user acts on is not the focus the system
holds. Landing on a project slot drills into the project's remembered content,
so a project can never be the subject of navigation itself.

## Decision

Keyboard navigation depends only on the keyboard-focused target. Zoom moves
focus where the framing crosses a project boundary, so the focused target is
always the thing the user is looking at: zooming out of a project focuses the
project, zooming in restores focus inside it.

### Zoom level

The zoom state is the keyboard-focused target plus a **zoom level** within the
target's **zoom project**:

```rust
enum ZoomLevel {
    Project,
    Row,
    Slot,
    Focus,
}
```

- The zoom project is the project whose matrix holds the focused target's slot.
  For a `Project(P)` target it is P's parent — P is focused as a slot of its
  parent. The root project, which no matrix hosts, is its own zoom project.
- `Focus` frames the focused target itself: an instance's panel, a launcher
  without instances, or a project's presented rect.
- `Slot` frames the slot containing the target (every visor of a launcher, a
  nested project's presented rect), `Row` the matrix row of that slot, and
  `Project` the zoom project.
- For a target that is not an instance, `Focus` and `Slot` frame the same rect;
  zoom commands treat them as one step.
- Navigation does not change the zoom level. Each level is resolvable on every
  target, so the level is stored as is and needs no clamp: at `Focus`, moving
  from an instance onto a project frames the project, and moving on to a
  launcher frames its instance again.

### Zoom commands

Zoom commands first normalize the stored level (`Slot` on a non-instance target
reads as `Focus`) so that every step changes the framing.

- **Zoom Out** steps `Focus → Slot → Row` (skipping `Slot` on a non-instance
  target). From `Row` it leaves the zoom project: focus moves to the
  `Project(zoom project)` target at `Focus`, which frames the same rect as the
  parent's slot. In the root project, `Row` steps to `Project` instead, keeping
  focus on the root slot; `Project` in the root is the outermost frame.
- **Zoom In** steps `Project → Row → Slot → Focus` (skipping `Slot` on a
  non-instance target). On a `Project(P)` target at `Focus` it enters P by one
  level: focus moves to P's **project focus slot** (its remembered slot, else
  its first slot in matrix order), resolved as navigation lands on a slot, at
  `Row`. Zoom In on an empty project does nothing.
- **Enter** — `Cmd+Enter`, and `Enter` without modifiers, on a project target —
  enters P all the way: focus follows the project focus slots through nested
  projects to a launcher's instance anchor (else the first launcher depth
  first) at `Focus`.
- A click, starting an instance, and `Cmd+Enter` on a non-project target set
  `Focus`, as before.
- A target is **fully zoomed in** when its normalized level is `Focus`.

### Navigation

- Arrows navigate from the focused target, regardless of the zoom level: an
  instance steps through its launcher's instances and then to neighboring
  slots, a launcher or project target steps between the slots of its zoom
  project.
- Landing on a slot focuses its content as a slot: a project slot focuses the
  `Project` target itself, never its content; a launcher slot focuses its
  instance anchor, else its directional edge instance, else the launcher.
- Vertical overflow past a project's matrix edge into sibling projects is
  unchanged; horizontal navigation does not overflow.
- Replacement focus after a slot is cleared lands on the neighboring slot by
  the same rule.

### Focus memory

Each project's focus slot is unchanged: it is recorded when the target in that
slot leaves the focus path. Zooming out of P records P's slot in its parent and
keeps P's own focus slot, which Zoom In and Enter restore.

### Indicator

The indicator shows what is framed: `Focus` on an instance reads `Instance`,
`Focus` on any other target reads `Slot`. The nesting depth is that of the zoom
project, root = 0.

## Consequences

- `ZoomDepth`, the zoom chain and the zoom index are removed;
  `CommitZoomDepth` becomes `CommitZoomLevel`, `ZoomLevel::Instance` becomes
  `ZoomLevel::Focus`, and `Zoom::Reset` becomes `Zoom::Enter`.
- A project target receives no text input; `Cmd+T` and `Cmd+W` do nothing on
  it.
- Navigating at `Focus` across a project slot zooms the camera out to the
  project and back in when focus reaches an instance again.
- ADR 0017's guarantee that navigation inside a zoomed-out frame does not move
  the camera no longer applies at `Focus`; at `Row` and `Project` it still
  holds as long as focus stays in the framed row or project.

## Considered options

- **Keep ADR 0017's depth and navigate at the framed granularity.** Rejected:
  arrow keys change meaning with the zoom depth.
- **A sticky `Innermost` level clamped to each target's levels.** Rejected: the
  stored value claims a level, such as `Instance`, the target may not have.
- **Zoom In descending to a leaf in one step.** Rejected: it is not the inverse
  of Zoom Out; Enter provides the direct descent instead.
- **Restoring focus from a stack of zoom-outs.** Rejected: after navigating to a
  sibling project the stack has no entry for it; the per-project focus slot
  does.
