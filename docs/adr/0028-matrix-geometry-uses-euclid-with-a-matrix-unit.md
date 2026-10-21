# Matrix geometry uses euclid with a matrix unit

## Status: accepted

Builds on [ADR 0025](./0025-slots-can-be-selected-beside-the-keyboard-focus.md), whose selection
types it replaces.

## Problem

The slot selection describes geometry in a project's matrix, counted in columns and rows. It uses
two hand-written types for that. `SelectionDelta` holds how far the selection reaches from the
focused slot, and `MatrixRect` holds the top-left and bottom-right placements of a rectangle in a
matrix, which the selection and the matrix layout share. Every operation on them is written by
hand: stepping the delta, spanning the rectangle, and soon testing which slots lie inside it, which
grouping a selection needs (ADR 0026).

massive already uses euclid for pixel and texture geometry, with a unit type that keeps the kinds of
coordinates apart, for example `SizePx`, `RectPx`, and `BoxPx` in `massive-geometry`.

## Decision

Matrix geometry gets its own euclid unit, `MatrixUnit`. Matrices are a concept of the desktop, so
the unit lives in the desktop crate next to `SlotPosition`, not in `massive-geometry`.

- The selection's delta becomes a `Vector2D<i32, MatrixUnit>`. The zero vector selects only the
  focused slot, and a step is a vector addition.
- `MatrixRect` becomes a `Box2D<u32, MatrixUnit>`. Containment and intersection come from euclid
  instead of being written by hand.
- `SlotPosition` stays as it is, because it is the persisted type and the configuration code uses
  it throughout. It converts to and from `Point2D<u32, MatrixUnit>` with `From` implementations.

`Box2D` treats its maximum as exclusive, while the selection includes both corners. The box
therefore stores the bottom-right placement plus one column and one row. Code that needs the last
selected column or row, such as the matrix layout's area of a selection, subtracts one.

## Consequences

- Matrix coordinates cannot be mixed up with pixel coordinates, because the unit types differ.
- The exclusive maximum is a place where an off-by-one can slip in. A test pins that a selection's
  box contains both of its corners and nothing beyond them.

## Considered options

- **Keep the hand-written types.** They need no conversions, but every geometric operation would be
  written and tested again by hand.
- **Convert `SlotPosition` to `Point2D` everywhere.** This would remove the conversions, but it
  touches the persisted configuration and much code unrelated to geometry, for a gain that only the
  selection needs today.
