# Keyboard focus changes are command-mediated

The event router no longer mutates keyboard focus inline. `process()` surfaces a *focus suggestion* (`ProcessOutcome::Focus { target, event }`), which the desktop system lowers into an explicit `DesktopCommand::NavigateTo { target, event }`. The actual focus change happens only when that command runs, and the focus-driven launcher relayout and visor-anchor sync follow at the transaction boundary (see Consequences), so input handling produces a single `Cmd` instead of a `(Cmd, Effects)` pair.

## Considered options

- **Return `(Cmd, Effects)` from input handling (previous design).** Rejected: input was the only producer of focus-relayout effects, giving it a dual return and a second, parallel path to the command/effect pipeline that programmatic focus already used.
- **Keep inline focus mutation but suppress effects via `CameraLocked` mode.** Rejected: `CameraLocked` is about camera motion, not focus relayout, and there is no marker distinguishing a focus-driven `Measure` from any other, so the deferral could not be expressed at effect-run time.

## Consequences

- All keyboard-focus changes — pointer click, window blur/restore — flow through `NavigateTo`. `target == None` means *remove focus only* (no camera/navigation effect).
- A `NavigateTo` carries the triggering click and delivers it to the new focus target after focusing, and therefore may itself yield follow-up commands (e.g. a launcher click producing `StartInstance`), which it applies recursively.
- The button-press deferral lives at the transaction boundary. Layout reads a *layout focus* (`DesktopSystem::layout_focus`), not the live keyboard focus. The layout focus catches up with the live focus only in a transaction that may move the camera, which is any transaction while no mouse button is pressed. When it changes, the launcher of the instance that now holds it takes that instance as its focus anchor, and the launchers of the previous and the new instance are re-measured if their layout depends on the focus. So a visor neither expands nor re-centers under the pointer during a press, even when something else re-lays it out meanwhile, and it catches up in the transaction of the release.
- Programmatic `focus()` and input `NavigateTo` share one source of truth for the anchor sync and the focus-driven relayout, the layout focus.
