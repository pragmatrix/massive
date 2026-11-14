# Launchers present only as visors

## Status: accepted

Amends [ADR 0019](./0019-instance-title-bar-is-part-of-the-instance-extent.md),
[ADR 0022](./0022-presenters-own-the-layout-of-their-targets.md),
[ADR 0023](./0023-matrix-slots-share-one-slot-extent-and-center-their-content.md),
[ADR 0037](./0037-desktop-configuration-is-kdl-reconciled-from-the-aggregate.md) and
[ADR 0041](./0041-a-view-requests-its-background-and-the-desktop-draws-it.md) where they describe
the band mode or a launcher that stretches.

## Problem

A launcher had a mode that decided how it presented its instances. A visor stands the instances on
an arc around the focus anchor, the instance that the visor centers on. A band packed them side by
side in a row. Every part of the desktop that lays out, frames, hit-tests or draws a launcher
branched on the mode, and the configuration file and the MCP tools carried it as a field. Only the
visor is in use, so the band cost a second path through all of these parts without serving anyone.

## Decision

- **A launcher always presents its instances as a visor.** The `LauncherMode` type and the band
  mode are removed. A launcher measures one slot extent whether it holds instances or not, places
  its instances on the visor's arc, and the camera frames a launcher's instances with the visor's
  arc camera.
- **A launcher is exactly one slot extent and never stretches.** Without the band, every launcher
  measures an instance extent, the default panel with its title bar (ADR 0019), which is exactly one
  slot extent (ADR 0023). Its matrix scale is therefore always 1. An empty launcher no longer
  stretches to its row, because a row holding a launcher is already one slot extent tall and
  nothing in a matrix is taller.
- **The configuration has no `mode`.** The desktop configuration does not write a launcher's mode
  and reads none. An existing file that still contains `mode` loads, because the configuration
  ignores fields it does not know, and the next write drops the field. A launcher saved as a band
  comes back as a visor. No migration is provided.
- **The MCP tools have no `mode`.** `LauncherSpec` and the launcher description lose the field. A
  client that still sends `mode` is rejected, because `LauncherSpec` denies unknown fields.
- **Every instance draws its background.** The instance background was drawn only for visors, so
  it is now unconditional, and hit testing always includes the children that overflow a launcher.

## Consequences

- The policies that hung off the mode (`launcher_mode.rs`) are gone. Their visor answers are now
  the only behavior.
- Only nested projects shrink to fit their slots and sit centered in them (ADR 0023). Launchers
  are never scaled or shifted in their slots.
- The gap between a launcher's instances, `CHILD_SPACING`, was zero and is removed.
- Moving the keyboard focus into a launcher still picks the focus anchor when the visor has one,
  and otherwise the edge instance that the direction enters from.
- The built-in default configuration, whose first launcher was a band, now starts with a visor.

## Considered options

- **Keeping `LauncherMode` with only the visor variant.** Rejected, because a field with a single
  value carries no information. A second presentation, if one returns, gets its own design.
- **Rejecting a configuration that still contains `mode`.** Rejected, because a stale field is
  harmless, and refusing to start the desktop over it would be.
