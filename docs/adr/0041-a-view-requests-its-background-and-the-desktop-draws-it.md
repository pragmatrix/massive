# A view requests its background, and the desktop draws it

## Status: proposed

Amended by [ADR 0044](./0044-launchers-present-only-as-visors.md): launchers present only as
visors, so every instance draws its background and `launcher_mode.rs` is gone.

## Context

A background has been drawn behind an instance's view since February 2026 (commit `f9bbd577`).
The instance presenter draws it as one rectangle in `colors::instance::BACKGROUND`, sized to the
primary view, and only for visor launchers (`renders_instance_background` in
`desktop/src/projects/launcher_mode.rs`). It belongs to the instance, so a view has no say in
it.

An application can already ask for a background color. `ViewBuilder::with_background_color`
stores the color, but `build` never passes it on, and `ViewCreationInfo` has no field for it. mt
asks for black (`src/main.rs`), and the request goes nowhere.

ADR 0040 hands the view location over to the instance. A desktop visual that refers to that
location by id could take the id of a location that an already ended instance had released. So the
instance background now hangs below a second location, which `InstanceRoot::new` creates for the
desktop and which shares the view transform. The background itself is still the instance's.

## Decision

- A background belongs to a view, not to an instance. The view requests its color with
  `ViewBuilder::with_background_color`, and `ViewCreationInfo` carries the color to the host.
- The desktop's `ViewPresenter` draws the background of each presented view, as a rectangle sized
  to the view, in the requested color. A view that requests no color has no background.
- The background hangs below a location the desktop owns. That location shares the view
  transform with the view location, so it sits exactly where the view sits, but it never refers to
  the view location the instance owns. The background can then be created and dropped together
  with its view presenter, at any time.
- The instance background and `renders_instance_background` go away.

## Open questions

- **Launcher modes.** Today only visors draw a background. Should a band draw a view's
  requested background, too, or does the launcher mode still decide whether any background is
  drawn?
- **The default color.** Visor instances show `colors::instance::BACKGROUND` today, although mt
  asks for black. Should views that request nothing get that color, so applications other than mt
  look as before, or no background at all?

## Considered Options

- **Keep the instance background below the desktop-owned location.** This is how ADR 0040 solves
  the lifetime of the background visual. It keeps a background that the view cannot influence.
- **Let the application draw its own background into the view.** That needs no new API, but every
  application would repeat it, although `ViewBuilder::with_background_color` already lets a view
  state the request.
