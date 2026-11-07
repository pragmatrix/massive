# 01 Pixel-perfect focus

Status: **experiment, not started** (2026-11-07). Part of [view columns](README.md).

## Question

Can text in a view stay pixel-perfect at the Focus zoom level while the instance's content is
scaled down to fit its panel?

The rest of the desktop design rests on the answer. The content extent of an instance with view
columns is usually larger than its panel, so the desktop scales the content down
([03](03-multi-view-instances.md)). The camera must then come closer by exactly the inverse of that
scale, so that the focused view appears at 1:1 on screen. Text in a view that has the keyboard focus
must never look blurry.

## Current situation

- Full Screen Mode already scales an instance's content, toward the window. Its scale comes from
  `fullscreen_scale` in
  [fullscreen.rs:30](../../desktop/src/desktop_system/fullscreen.rs#L30), which fits one size into
  another.
- The glossary says that a focused instance presented at its Full Screen scale "resolves as
  pixel-perfect at that scale automatically". It is not yet known how this works, or whether it
  carries over to a scale that the camera cancels.
- The camera zooms by distance, not by scale ([ADR 0004](../adr/0004-camera-zoom-lives-in-distance-not-scale.md)).
  The camera frames the keyboard-focused target
  ([ADR 0016](../adr/0016-camera-zoom-is-relative-to-keyboard-focus.md),
  [ADR 0018](../adr/0018-zoom-is-a-level-relative-to-the-focused-target.md)).

## Experiment

1. Find how Full Screen Mode keeps text pixel-perfect, and write it down here.
2. Present an existing single-view instance, such as a terminal, at an artificial content scale
   below 1, for example 0.5 and 0.37.
3. Frame it at the Focus zoom level with the camera distance that cancels the scale.
4. Compare screenshots with the same view at scale 1.

## Done when

The screenshots match the unscaled rendering pixel for pixel, at every scale tried. If they do not
match, this document records why, and the desktop design in [03](03-multi-view-instances.md) is
revised before any other step starts.

## Open questions

- Whether rounding of the camera distance can shift text by a fraction of a pixel, and how to snap
  it.
- Whether glyphs are shaped at the presented size or at the view's own size.
