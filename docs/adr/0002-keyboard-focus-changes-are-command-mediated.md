# Keyboard focus changes are command-mediated

The event router no longer mutates keyboard focus inline. `process()` surfaces a *focus suggestion* (`ProcessOutcome::Focus { target, event }`), which the desktop system lowers into an explicit `DesktopCommand::NavigateTo { target, event }`. The actual focus change, focus-driven launcher relayout, and visor-anchor sync happen only when that command runs, so input handling produces a single `Cmd` instead of a `(Cmd, Effects)` pair.

## Considered options

- **Return `(Cmd, Effects)` from input handling (previous design).** Rejected: input was the only producer of focus-relayout effects, giving it a dual return and a second, parallel path to the command/effect pipeline that programmatic focus already used.
- **Keep inline focus mutation but suppress effects via `CameraLocked` mode.** Rejected: `CameraLocked` is about camera motion, not focus relayout, and there is no marker distinguishing a focus-driven `Measure` from any other, so the deferral could not be expressed at effect-run time.

## Consequences

- All keyboard-focus changes — pointer click, window blur/restore — flow through `NavigateTo`. `target == None` means *remove focus only* (no camera/navigation effect).
- A `NavigateTo` carries the triggering click and delivers it to the new focus target after focusing, and therefore may itself yield follow-up commands (e.g. a launcher click producing `StartInstance`), which it applies recursively.
- The button-press deferral now lives at the transaction boundary: the visor anchor sync and the flush of deferred focus relayout are gated on `any_buttons_pressed()` (authoritative device state), not on the transaction's effects mode.
- Programmatic `focus()` and input `NavigateTo` share one source of truth for anchor sync and relayout flushing.
