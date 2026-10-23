# A project's header does not widen its project

## Status: accepted

Builds on [ADR 0023](./0023-matrix-slots-share-one-slot-extent-and-center-their-content.md), whose
slot extent, shared project scale and stretching stay in force.

## Problem

A project's header shows its name in a large font. The header measured the full width of that
name, and the project measured as the wider of its header and its matrix. A name of a dozen
characters was wider than a slot. It made its project wider than the matrix, which then sat
centered under the name. Through the shared project scale of ADR 0023, it also shrank every
sibling project.

## Decision

- **The header measures no width.** It still stretches across the project's width and elides the
  start of a name that does not fit. A project's width therefore comes from its matrix and its
  padding, and grows only when its parent stretches it to a slot. A name never changes the size of
  its project or of its siblings.
- **The horizontal alignment of a matrix's columns is a policy.** When a stretched project is wider
  than its matrix's columns, the constant `matrix::HORIZONTAL_ALIGNMENT` in
  `desktop/src/policy/constants.rs` decides where they sit: leading, centered or trailing. It is
  leading, so the columns start under the start of the header. This replaces ADR 0023's horizontal
  centering of the intervals. Vertically, the rows stay centered, and each slot's content stays
  centered in its slot.

## Consequences

- A long name is elided instead of making room for itself.
- The selection outline (ADR 0025) and the camera framing (ADR 0032) follow the aligned columns,
  because they read the same interval origin as the slots.
- The alignment is a tuning value, not an exceptional behavior, so it is a constant and not a
  `when` rule (ADR 0029).

## Considered options

- **Leading alignment only for a project whose header is wider than its matrix.** Rejected,
  because the header should not size the project in the first place.
- **Clipping the name instead of eliding it.** Rejected for now. Eliding is the title bar's default
  behavior and needs no change.
