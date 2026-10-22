# Configuration operations are one command vocabulary

## Status: accepted, implemented

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), whose change
vocabulary it layers beneath.

## Problem

The desktop's configuration can be changed in three ways. Keyboard shortcuts arrive as
`DesktopCommand`s, the CLI (`mt`) sends `ConfigurationRequest`s from inside an instance, and the
boot replay rebuilds the scene from `ConfigurationCommand`s. Each of them spells out its own set
of operations, and each one reaches into a different level of the planning code.

`ConfigurationCommand` only knows four operations: assign, clear, move, and set the startup path.
Every operation added later went around it. Adding next to a slot, grouping, ungrouping, swapping,
and pushing are planned by separate functions or emitted as raw `ConfigurationChange`s, and each
is reachable only from the one that first needed it. The CLI therefore cannot group, and
the keyboard cannot push. The policies drift as well. Removing by key refuses while instances run,
removing by CLI stops them, and only some operations decide where the focus goes afterwards.

## Decision

The keyboard, the CLI, and the boot replay each do two separate jobs, and they are kept apart:

1. **Naming the slots** decides which slots a command acts on. The keyboard takes them from the
   selection, which always contains the focused slot. The CLI names them by names and target
   paths, relative to the instance it runs in. The boot replay names them by ids.
2. **The operation** decides what happens to the named slots.

`ConfigurationCommand` becomes the one complete vocabulary of operations. It contains every
operation a user can perform on the configuration: assign, clear, move, swap, push, add next to,
group, ungroup, and set the startup path. A command names its slots by ids and slot positions only,
never by focus or by names. A move only lands on an unassigned slot, so it never displaces
content. Exchanging two assigned slots is a swap.

Each command is planned in exactly one place, which expands it into the primitive
`ConfigurationChange`s of ADR 0011 together with the topology and focus changes it needs.

The keyboard, the CLI, and the boot replay only name their slots by ids and slot positions and
issue a `ConfigurationCommand`. A desktop policy guard that applies to one of them decides whether
the command is issued at all. Cmd+Shift+W, for example, issues no removal while instances run below
the focused slot, so that a key press never quits running processes. A removal that is issued
always stops the instances below it, which is what the CLI's `remove` does.

The operation decides where the focus goes afterwards. A grouping therefore ends the same way
whether it came from Cmd+G or from `mt`.

## Consequences

- The configuration variants of `DesktopCommand` collapse into `ConfigureSelection`, which applies a
  `SelectionOperation` to the selection. A key cannot know which slots the selection covers, so the
  desktop system builds the command the operation stands for.
- Each `ConfigurationRequest` reduces to resolving its names and target paths, then issuing a
  command. Every operation is open to the CLI and to the keyboard: giving one of them an operation
  it lacks needs only the code that names its slots. The CLI does not group or ungroup yet, and no
  key pushes or moves a slot yet.
- `ConfigurationChange` stays the primitive vocabulary that is applied. Commands and changes are
  different levels: a command is what a user asks for, and its changes are the steps that carry it
  out.

## Considered options

- **Unify only the planning.** Every operation would get a `ConfigurationCommand` and one planner,
  but `DesktopCommand` and `ConfigurationRequest` would keep their own operation variants and
  forward them. This changes less code, but it leaves three enums that have to be kept in step,
  which is the drift this decision removes.
