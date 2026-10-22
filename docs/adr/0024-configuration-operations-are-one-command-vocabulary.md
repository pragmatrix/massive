# Configuration operations are one command vocabulary

## Status: accepted, not implemented yet

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), whose change
vocabulary it layers beneath.

## Problem

The desktop's configuration can be changed from three front ends. Keyboard shortcuts arrive as
`DesktopCommand`s, the CLI (`mt`) sends `ConfigurationRequest`s from inside an instance, and the
boot replay rebuilds the scene from `ConfigurationCommand`s. Each front end spells out its own set
of operations, and each one reaches into a different level of the planning code.

`ConfigurationCommand` only knows four operations: assign, clear, move, and set the startup path.
Every operation added later went around it. Adding next to a slot, grouping, ungrouping, swapping,
and pushing are planned by separate functions or emitted as raw `ConfigurationChange`s, and each
is reachable only from the front end that first needed it. The CLI therefore cannot group, and
the keyboard cannot push. The policies drift as well. Removing by key refuses while instances run,
removing by CLI stops them, and only some operations decide where the focus goes afterwards.

## Decision

Every front end does two separate jobs, and they are kept apart:

1. **Resolving the target** decides which slot is meant. The keyboard targets the focused slot.
   The CLI names slots by names and target paths, relative to the instance it runs in. The boot
   replay names them by ids.
2. **The operation** decides what happens to the targeted slot.

`ConfigurationCommand` becomes the one complete vocabulary of operations. It contains every
operation a user can perform on the configuration: assign, clear, move, swap, push, add next to,
group, ungroup, and set the startup path. A command names its slots by ids and placements only, never
by focus or by names.

Each command is planned in exactly one place, which expands it into the primitive
`ConfigurationChange`s of ADR 0011 together with the topology and focus changes it needs.

A front end only resolves its target into ids and placements, chooses the policies that may
differ between front ends, and issues a `ConfigurationCommand`. Such a policy rides on the command,
as `SlotShift` already does. One example is whether a removal refuses or stops running instances.

The operation decides where the focus goes afterwards. A grouping therefore ends the same way
whether it came from Cmd+G or from `mt`.

## Consequences

- The configuration variants of `DesktopCommand` (`AddLauncher`, `GroupSlot`, `UngroupSlot`,
  `RemoveSlot`) collapse into one variant that applies an operation to the focused slot.
- Each `ConfigurationRequest` reduces to resolving its names and paths, then issuing a command. The
  CLI gains every operation the keyboard has, and the other way round.
- `ConfigurationChange` stays the primitive vocabulary that is applied. Commands and changes are
  different levels: a command is what a user asks for, and its changes are the steps that carry it
  out.

## Considered options

- **Unify only the planning.** Every operation would get a `ConfigurationCommand` and one planner,
  but `DesktopCommand` and `ConfigurationRequest` would keep their own operation variants and
  forward them. This changes less code, but it leaves three enums that have to be kept in step,
  which is the drift this decision removes.
