# Desktop notifications plan

Status: **need only, no design yet** (2026-10-20). This document records why the desktop needs a
way to show short, transient messages, often called toasts. It collects the cases that need one
and the requirements known so far. The design is still open.

## Goal

When the desktop refuses or ignores a user action, it should tell the user why. Today it can
only write a log line, which the user does not see. A key press that does nothing looks like a
bug, and the user has no way to learn what they would have to do instead.

## Current situation

Some desktop actions are deliberately refused, and the refusal is silent on screen:

- **Removing a launcher with running instances.** The planned launcher-removal shortcut only
  removes a launcher that has no instances, so that a key press never quits running shells and
  their processes. When instances are running, the shortcut does nothing. The user should learn
  that the launcher was kept because of its instances, and that they can close those first.
- **Grouping without a slot.** Cmd+G with the focus outside any slot, for example on the root
  project, only logs "The focus sits in no slot to group".
- **Moving a launcher off the matrix.** A directional launcher move past the matrix edge only
  logs that it was ignored.

More cases will come as the desktop gains commands that act on the configuration.

## Requirements known so far

- **Non-modal.** A notification never takes the keyboard focus and never blocks input. The user
  keeps working while it is shown.
- **Transient.** It disappears on its own after a short time. A new notification for the same
  cause replaces the old one instead of stacking a copy.
- **Readable at every zoom level.** The desktop is a zoomable 3D scene, so a notification placed
  in the scene could be too small or off screen. It most likely belongs in window space, in front
  of the scene, at a fixed size.
- **Close to its cause, where possible.** A notification about a launcher should make clear which
  launcher it is about, for example by its name.
- **Reachable from planning.** Refusals are decided while a command is planned, so planning needs
  a way to emit a notification, much like it emits changes today.

## Open questions

- Where notifications appear in the window, and how several are arranged.
- Whether a notification can carry an action, such as "Close instances".
- Whether notifications are also kept somewhere the user can look them up later.
- Whether refusals of configuration requests from the CLI (`mt`) are shown as notifications too,
  or only reported to the CLI.
