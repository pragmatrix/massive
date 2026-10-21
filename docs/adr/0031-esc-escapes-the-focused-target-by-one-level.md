# Esc escapes the focused target by one level

## Status: accepted

Builds on [ADR 0018](./0018-zoom-is-a-level-relative-to-the-focused-target.md), whose Zoom Out it
shortens, and on [ADR 0030](./0030-the-selection-surrounds-the-focused-slot.md), whose Esc it
extends.

## Problem

Enter on a focused project enters it (ADR 0018), but nothing leaves a project in one key. Zoom Out
(Cmd+Ctrl+↓) steps through the zoom levels and leaves the project only after two or three presses.
Sketch, Keynote, and Illustrator leave a group with Esc, so users reach for Esc first.

Esc also belongs to the applications. In a terminal, vim leaves insert mode with it, readline uses
it as the Meta prefix, and most full-screen programs cancel with it. The desktop must never take
Esc from an application that has the focus.

## Decision

**Esc escapes the focused launcher or project by one level.** Like navigation, which escapes to
the slot hosting a project when its matrix has no neighbor, the project whose matrix holds the
focused slot takes the focus at the zoom level `Focus`, where Zoom Out lands when it leaves a
project. On a focused nested project, Esc therefore escapes to the project that holds it. Enter
goes back to the remembered slot. In the root project, which has no parent, Esc frames the whole
root, and there it stops.

**Esc collapses an extended selection first** (ADR 0030). Only the next Esc escapes.

**Plain Esc stays with a focused instance.** The key reaches the application, as before.

**Cmd+Esc is the desktop's Esc.** It does what Esc does, and on a focused instance it also escapes
the instance: the instance's launcher takes the focus, and the zoom level stays. The desktop
already takes Cmd+arrows, Cmd+Enter, Cmd+T, and Cmd+W from instances, and no known terminal
program uses Cmd+Esc.

The desktop policy decides between collapsing, leaving, and passing the key on
(`when::pressing_escape`, ADR 0029).

## Consequences

- Pressing Cmd+Esc repeatedly escapes from anywhere to the root's outermost frame. Plain Esc
  escapes the same way once the focus is outside an instance.
- A stray Esc on a launcher moves the camera out to the enclosing project. Enter brings it back.
- An application that enabled the Kitty keyboard protocol does not receive Cmd+Esc. Passing it on
  only to applications that do not use it would need a way for an application to tell the desktop,
  which key delivery does not have (ADR 0020).

## Considered options

- **Esc takes one Zoom Out step.** Rejected, because leaving a project would take two or three
  presses, and the key would duplicate Cmd+Ctrl+↓.
- **Cmd+Esc escapes an instance only for applications without the Kitty protocol.** Rejected for
  now, because the desktop decides before the application sees the key and cannot know which
  protocol the application enabled.
