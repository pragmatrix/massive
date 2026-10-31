# Desktop MCP tools

Status: **decided** (2026-10-24). `desktop_describe` and the slot tools are implemented (2026-10-31).

This document specifies the MCP tools the desktop offers to agents through an MCP root
([ADR 0036](adr/0036-an-mcp-root-exposes-a-project-subtree-to-agents.md)). The desktop is a tool
provider with the MCP id `desktop`, so every tool's name starts with `desktop_`. An agent sees these
names with its client's prefix, for example `mcp__<server>__desktop_describe` in Claude Code.

## Conventions

**Every call acts inside the connection's MCP root.** Every id a call names must belong to a
project, launcher or instance that the MCP root covers. Every slot it names, as source and as
destination, must lie in a covered project's matrix. The desktop checks both before anything
happens, together with the tool's access scope.

**Ids are session ids.** Projects, launchers and instances are named by UUID strings, as
`desktop_describe` reports them. An id is valid only while the desktop runs, because ids are
created anew every session. A slot is named by its project's id and its slot position.

**A tool never moves the user's focus.** Several configuration commands choose where the focus goes
afterwards, when the keyboard issues them (ADR 0024). A command issued through MCP carries the focus
intent `Keep` instead, so the desktop leaves the keyboard focus, the selection and the camera where
they are. The one exception is a focused target that the command removes. The focus then moves as
it does for the keyboard, because it cannot stay on something that no longer exists.

**Configuration tools return only what they create.** A tool that creates a launcher, a project or
an instance returns its id, and every other tool returns an empty object. An agent that needs the
new arrangement calls `desktop_describe`.

**Each tool shows its signature in pseudo-JSON.** The block names the tool, lists its parameters
in braces, and shows its result after `→`. A field marked `?` is optional, `|` separates the values a
field can take, and `[x, ...]` is an array of `x`. A capitalized name refers to a shared type below.
A `//` comment says in a few words what a field holds. The tables below the signature give each
field's limits, default and meaning in full.

**Every field names where it comes from.** A field's comment ends with its source in brackets, and
fields of the form `[M]` have a rationale below their tables:

| Tag | Source |
|---|---|
| C | A field of the `ConfigurationCommand` or `DesktopCommand` the tool issues (ADR 0024). |
| F | A field of the desktop configuration document (ADR 0013). |
| A | Runtime state the desktop keeps for the session, such as instances and their titles. |
| M | Added for MCP. None of the commands or documents has it. |

**Each tool names what it issues.** Its **Issues** line names the command the tool turns into, and
its **Keyboard** line names the key that issues the same command, if one does.

**Every result is structured.** Each tool declares an MCP output schema and returns its result as
`structuredContent`, repeated as a text block for clients that do not read structured content.

**A failed call is an MCP tool error.** Its result has `isError` set and one text block that starts
with an error code, a colon and a space, followed by a sentence for the agent. These codes exist:

| Code | Meaning |
|---|---|
| `unknown_id` | An id names no project, launcher or instance that the MCP root covers. It may be stale, or it may lie outside the root, which the error does not reveal. The text names the id. |
| `scope_not_granted` | The MCP root does not grant the tool's access scope, or the command would stop running instances without `act`. |
| `invalid_argument` | An argument breaks the schema's rules, such as a slot position outside the matrix. A slot lies at most one column right of and one row below the matrix's assigned slots. |
| `refused` | The desktop refused the command, and the text gives the reason. Examples are clearing an unassigned slot, a push past the matrix's edge, a move onto an assigned slot of another project, a move into the content's own subtree, nesting a project more than 16 levels below the root project, nesting one MCP root inside another, and ungrouping an MCP root. |
| `too_large` | The call's arguments or its result exceed the tool protocol's frame limit of 1 MiB. Only this call fails. |
| `connection_closed` | The tool connection ended before the desktop answered the call, for example because the MCP root's configuration changed. |

## Shared types

**`SlotPosition`**: a slot in a project's matrix.

```text
SlotPosition = {
  column: integer,  // counted from 0 [C]
  row: integer      // counted from 0 [C]
}
```

**`SlotRef`**: a slot in a named project.

```text
SlotRef = {
  project_id: string,      // the project whose matrix holds the slot [C]
  position: SlotPosition   // the slot [C]
}
```

**`LauncherSpec`**: the launcher a tool creates.

```text
LauncherSpec = {
  name: string,                                 // shown in the title bar [F]
  mode?: "visor" | "band",                      // default "visor" [F]
  params?: object,                              // passed to the application [F]
  full_screen_mode?: "regular" | "fullscreen"   // default "regular" [F]
}
```

| Field | Type | Meaning |
|---|---|---|
| `name` | string, 1 to 1000 characters | The launcher's name. Names may repeat. |
| `mode` | `"visor"` or `"band"` | How the launcher presents its instances. The default is `"visor"`. |
| `params` | object | The parameters the launcher's application receives when an instance starts, such as a command to run. Their meaning belongs to the application. They nest at most 4 levels deep. Setting them needs `act` on top of the tool's scope, because they decide what the user runs by starting an instance. |
| `full_screen_mode` | `"regular"` or `"fullscreen"` | The Full Screen Mode the launcher's primary instances share (ADR 0014). The default is `"regular"`. |

**`Content`**: what a tool puts into a slot. Exactly one field is set.

```text
Content = {
  launcher?: LauncherSpec,  // a new launcher [C]
  project?: string          // the name of a new, empty project, 1 to 1000 characters [C]
}
```

**`Direction`**: `"left" | "right" | "up" | "down"` [C].

**`Shift`**: how a tool treats the slots next to the one it changes [C].

| Value | Meaning |
|---|---|
| `"shift"` | Assigning pushes the assigned content and its contiguous run one column right. Clearing pulls the rest of the row left. |
| `"keep"` | Assigning onto an assigned slot replaces its content, and clearing leaves the slot unassigned. |

**Defaults do less.** Each tool defaults to the value that removes nothing and moves the least:
`"shift"` for assigning, because `"keep"` would remove the content in the way, and `"keep"` for
clearing, because `"shift"` would move the rest of the row and change positions the agent knows.

**`Project`**, **`Slot`**, **`Launcher`** and **`Instance`**: the covered subtree as
`desktop_describe` returns it.

```text
Project = {
  id: string,          // session id [A]
  name: string,        // names may repeat [F]
  slots: [Slot, ...]   // assigned slots only [F]
}

Slot = {
  position: SlotPosition,     // where the slot sits [F]
  launcher: Launcher | null,  // set for a launcher slot [F]
  project: Project | null     // set for a nested project [F]
}

Launcher = {
  id: string,                                  // session id [A]
  name: string,                                // as in LauncherSpec [F]
  mode: "visor" | "band",                      // as in LauncherSpec [F]
  params: object,                              // as in LauncherSpec [F]
  full_screen_mode: "regular" | "fullscreen",  // as in LauncherSpec [F]
  instances: [Instance, ...]                   // its running instances [A]
}

Instance = {
  id: string,                     // session id [A]
  kind: "primary" | "assistant",  // how it was started [A]
  title: string,                  // the title bar's title [A]
  mcp_id: string | null           // its application's tools [M]
}
```

Exactly one of `Slot.launcher` and `Slot.project` is set. An `Instance.title` is the title of the
instance's primary view, which the instance title bar shows. It is empty while the application has
set none.

**Why the desktop adds its own fields:**

- `mcp_id`: It tells the agent which application tools the instance offers, because their names
  start with this id. Null means the instance's application offers no tools.

## Tools

### `desktop_describe`

Access scope `observe`. Returns the MCP root and everything it covers. It names nothing above the MCP
root, not even its target path, because a connection learns nothing about what lies outside it.

**Issues:** nothing. It reads the desktop's state. **Keyboard:** none.

```text
desktop_describe {}
→ Project   // the MCP root and its subtree [A F]
```

The result is the MCP root as a `Project`, with all its slots, nested projects, launchers and
running instances. The agent can name the project by its `name` when it talks to the user.

### `desktop_assign_slot`

Access scope `organize`. Puts a new launcher or a new, empty project into a slot.

**Issues:** `ConfigurationCommand::AssignSlot`. **Keyboard:** none. Cmd+N adds a launcher right of
the focused slot, which is an assignment one column right of it with `shift: "shift"`.

```text
desktop_assign_slot {
  slot: SlotRef,    // where to put the content [C]
  content: Content, // the new launcher or project [C]
  shift?: Shift     // default "shift" [C]
}
→ {
  id: string        // the new launcher's or project's id [M]
}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `slot` | `SlotRef` | yes | The slot to assign. |
| `content` | `Content` | yes | What to put there. |
| `shift` | `Shift` | no | What happens to content in the way. The default is `"shift"`. |

Replacing assigned content with `shift: "keep"` removes the old content and its subtree. When
instances run below it, the call also needs `act`.

**Why the desktop adds its own fields:**

- `id`: The desktop creates the new content's id, so the agent learns it only from the result. It
  can then act on the new content without describing the whole subtree again.

### `desktop_clear_slot`

Access scope `organize`. Removes a slot's content and its subtree.

**Issues:** `ConfigurationCommand::ClearSlot`. **Keyboard:** Cmd+Shift+W, which refuses while
instances run below the content.

```text
desktop_clear_slot {
  slot: SlotRef,   // the slot to clear [C]
  shift?: Shift    // default "keep" [C]
}
→ {}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `slot` | `SlotRef` | yes | The assigned slot to clear. An unassigned slot is refused. |
| `shift` | `Shift` | no | Whether the rest of the row moves into the cleared slot. The default is `"keep"`. |

Clearing stops every instance that runs below the content. When any run, the call also needs `act`.

### `desktop_move_slot`

Access scope `organize`. Moves a slot's content to another slot, possibly in another covered
project. Onto an assigned slot of the same project, it exchanges the two contents, so an agent only
names where the content goes.

**Issues:** `ConfigurationCommand::MoveSlot`, which exchanges the two contents when the
destination is assigned. **Keyboard:** none.

```text
desktop_move_slot {
  from: SlotRef,   // the content's slot [C]
  to: SlotRef      // where the content goes [C]
}
→ {}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `from` | `SlotRef` | yes | The assigned slot whose content moves. |
| `to` | `SlotRef` | yes | The destination. An assigned destination in the same project exchanges the two contents, and one in another project is refused. It must not lie inside the moved content's own subtree. |

### `desktop_push_slot`

Access scope `organize`. Pushes a slot's content one slot in a direction, together with the
contents in its way.

**Issues:** `ConfigurationCommand::PushSlot`. **Keyboard:** none.

```text
desktop_push_slot {
  slot: SlotRef,         // the content to push [C]
  direction: Direction   // where it moves [C]
}
→ {}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `slot` | `SlotRef` | yes | The assigned slot to push. A push that would move content past the matrix's left or top edge is refused. |
| `direction` | `Direction` | yes | The direction of the push. |

### `desktop_group`

Access scope `organize`. Wraps slots of one project into a new project, which takes the top-left
slot of their bounding box (ADR 0026).

**Issues:** `ConfigurationCommand::Group`, without its focus change. **Keyboard:** Cmd+G, on the
selected slots.

```text
desktop_group {
  project_id: string,              // the project whose matrix holds the slots [C]
  positions: [SlotPosition, ...],  // the slots to wrap [C]
  name: string                     // the new project's name [C]
}
→ {
  id: string                       // the new project's id [M]
}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `project_id` | string | yes | The project. |
| `positions` | array of `SlotPosition`, at least one | yes | The slots to wrap. Unassigned slots among them are ignored. |
| `name` | string, 1 to 1000 characters | yes | The new project's name. |

**Why the desktop adds its own fields:**

- `id`: As for `desktop_assign_slot`.

### `desktop_ungroup`

Access scope `organize`. Moves a nested project's slots into its parent's matrix and removes the
project (ADR 0026). An empty project is removed instead (ADR 0029).

**Issues:** `ConfigurationCommand::Ungroup`, without its focus change. **Keyboard:** Cmd+Shift+G.

```text
desktop_ungroup {
  project_id: string   // the nested project to dissolve [C]
}
→ {}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `project_id` | string | yes | A nested project below the MCP root. The MCP root itself cannot be ungrouped, because its slots would leave the covered subtree, and the desktop policy refuses it anyway. |

### `desktop_start_instance`

Access scope `act`. Starts a primary instance of a launcher, with the launcher's parameters.

**Issues:** `DesktopCommand::StartInstance`, as a primary instance and without its focus change.
**Keyboard:** Enter on a launcher, or Cmd+T.

```text
desktop_start_instance {
  launcher_id: string   // the launcher to start from [C]
}
→ {
  id: string            // the new instance's id [M]
}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `launcher_id` | string | yes | The launcher. |

Starting an instance runs the launcher's application with the launcher's parameters, which can run
a command. That is why it needs `act`.

**Why the desktop adds its own fields:**

- `id`: The agent needs the new instance's id to call its application's tools.

### `desktop_stop_instance`

Access scope `act`. Stops an instance and ends its application.

**Issues:** `DesktopCommand::StopInstance`. **Keyboard:** Cmd+W.

```text
desktop_stop_instance {
  instance_id: string   // the instance to stop [C]
}
→ {}
```

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `instance_id` | string | yes | The instance. |
