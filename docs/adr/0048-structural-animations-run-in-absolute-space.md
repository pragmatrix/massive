# Structural animations run in absolute space

## Status: proposed

## Context

Every presenter of slot content animates its own placement. A launcher, a project and a project's
matrix each own a movement whose value is a `SlotLayout`
([slot_layout.rs](../../desktop/src/projects/slot_layout.rs),
[project_presenter.rs](../../desktop/src/projects/project_presenter.rs)). The value is the transform
relative to the parent's scene location, and the renderer composes these local transforms along the
location tree into what is presented
([location_matrices.rs](../../renderer/src/scene/location_matrices.rs)). So what the user sees is
the product of several independent animations, one per level of nesting.

That product misbehaves in three ways, all measured on a project with two launchers side by side
that is grouped and ungrouped. The measurements traced each launcher's presented transform as the
renderer resolved it, frame by frame.

- **Opposite scales swell.** On ungrouping, the launchers grow from scale 0.497 to 1.0 inside their
  project, while the project shrinks from 0.991 to 0.497. Interpolated linearly, both ends show the
  launchers at about 0.49, but halfway through they show at 0.56, 13% larger, and they drift
  sideways and back. Interpolating scales by ratio and transforms as a zoom about their fixed point
  ([interpolatable.rs](../../animation/src/interpolatable.rs)) removes this, but only while all
  levels animate on the same curve.
- **Interruptions diverge.** When a change interrupts a running animation, an existing level blended
  its old animation with the new one, while a re-parented or new level started fresh. The curves
  diverged, and the launchers swung between scale 0.36 and 0.68. Replacing a running animation
  instead of blending it (`SlotLayout::set`) keeps the levels on the same curve, at the price of an
  abrupt change of speed on every interruption.
- **Re-parenting needs a conversion.** A launcher that moves into another project's matrix keeps its
  local transform, which the new parent then reads in its own coordinates. Re-parenting therefore
  converts the presented placement from the previous parent's coordinates into the new one's
  ([reparenting.rs](../../desktop/src/desktop_system/reparenting.rs)). The conversion has to use the
  transforms the parents present at that moment, not their placements, because a parent may still be
  animating. Project and matrix presenters read it from their movements for that purpose
  (`presented_transform`). A new group starts its matrix at the inverse of its own start, so that
  its slots need no conversion when they move in (`ProjectPresenter::animate_in_from`).

All three fixes are in place, and each one patches a consequence of the same cause: the animations
run in local coordinates, but what the user should see move smoothly is the absolute position.

## Decision

Structural animations of scene locations run in absolute space, in one animation controller near the
renderer. The scene keeps its location tree, and presenters keep setting local transforms relative
to their parents.

- **Presenters set targets, not animations.** A presenter sets a location's local transform and
  says whether the change animates. It no longer owns a movement for its placement.
- **The controller animates absolute transforms.** For each location whose transform changes with
  an animation, the controller takes the absolute transform presented at that moment as the start.
  It composes the new local transform with the parent's target to get the absolute target. It
  interpolates between the two and presents the result. A location without an animation of its own
  rides on its parent, as today.
- **Re-parenting keeps the presented position by itself.** Moving a location under another parent
  changes neither its presented absolute transform nor the start of its next animation. The
  controller only sees a new parent and a new local target.
- **Each location's path is its own.** No presented transform is the product of several running
  animations, so opposite scales, interrupted levels and re-parented levels cannot swing anymore.
  Blending stays possible, because it only affects the one location.

The controller lives where locations are resolved into presented transforms, in the renderer's
location resolution. That code already walks the location tree from the root down and composes
transforms per frame. Alpha fades, colors and other values that do not nest stay in the presenters'
movements.

## What this simplifies

Implementing this decision removes the following, which exists only because animations run in
local coordinates:

- The conversion on re-parenting: `SlotLayout::rebase`, `reparent` on `LauncherPresenter` and
  `ProjectPresenter`, and `presented_matrix_transform` and `previous_to_current` in
  [reparenting.rs](../../desktop/src/desktop_system/reparenting.rs). `reparenting::apply` would only
  move the hierarchy node and attach the scene location to the new matrix.
- `presented_transform` on project and matrix presenters, and `MovementRuntime::inspect`, through
  which they read their presented placement for the conversion.
- The matrix's start at the inverse of its project's start (`matrix_start`). A new group would start
  at the bounds of its slots, and its matrix at its placement, like any other location.
- The rule that `SlotLayout` replaces a running animation instead of blending it. Each location
  could blend again, and an interruption would no longer change the speed abruptly.
- The fixed-point zoom and the ratio interpolation as a correctness measure. They could stay because
  ratio steps look even when zooming, but no product of animations depends on them anymore.
- The per-presenter placement movements and `SlotLayout` itself, with their mounting, their
  `set_layout` helpers, and the first-placement rule that each presenter repeats.

## Feasibility

The renderer already resolves every location's transform per frame and knows each location's
parent, so it can compose start and target transforms where it composes presented ones today. What
has to move or be added:

- **The animation clock.** The renderer has to animate with the time the desktop's animation
  coordinator gives a frame, so that structural animations and movements stay in step
  ([ADR 0008](archive/0008-task-local-ui-contexts.md)).
- **Requests for frames.** Today a running movement keeps frames coming. A running structural
  animation in the controller has to do the same.
- **Snapping.** A transaction that snaps instead of animating, such as a window resize, has to reach
  the controller with each change.
- **Completion.** Nothing in the desktop reads the end of a placement animation today, so no
  completion event has to move along.

Hit testing and the camera read placements, not presented transforms, so they stay as they are. The
camera's zoom is not a location in the tree, so the screen still shows the camera's animation
multiplied by the placements'. Interpolating the camera's distance by ratio keeps that product even.

## Considered options

- **A flat scene for slot content.** Launchers and projects would hang directly under the desktop's
  location with absolute transforms, which solves the same problems. But every change of a project
  would have to update every slot below it, because nothing rides on its parent anymore, and the
  scene would lose the structure that hit testing and the renderer use.
- **Keep the local animations with their fixes.** This is the state today, and it works. Each new
  kind of nesting or interruption can bring another case like the three above.
