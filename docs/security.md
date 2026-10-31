# Security

This document tracks everything security-relevant in massive.
It states the trust model, lists every security check and where it lives in the code, and keeps the
known gaps visible. Read it before changing anything that lets a process outside the desktop act on
the desktop, and update it in the same change.

## Trust model

The desktop trusts the user at the keyboard completely. Every other actor is an agent or a process
that acts on the user's behalf, and it gets only what the user granted.

All processes of one user can reach the same files and sockets. The operating system cannot tell
them apart without a sandbox, and massive does not sandbox anything. A secret such as a token would
be readable by the same processes it is meant to keep out. Massive's checks therefore limit what a
process can do **through a channel massive controls**, such as MCP. They cannot limit what a process
does on its own.

The MCP bridge is no security boundary. Any same-user process can open the desktop
socket and speak the tool protocol without it. The boundary is the desktop's tool host, which
decodes and checks everything that arrives on a tool connection.

## Surfaces

| Surface | Who uses it | Decided in |
|---|---|---|
| The desktop socket in `~/.massive/run/` | The MCP bridge, and any tool client | [ADR 0036](adr/0036-an-mcp-root-exposes-a-project-subtree-to-agents.md) |
| The file descriptor an instance inherits | The CLI | [ADR 0011](adr/0011-desktop-is-a-fractal-of-projects-in-slots.md), [ADR 0024](adr/0024-configuration-operations-are-one-command-vocabulary.md) |
| `~/.massive/desktop.json` | The user, any same-user process | [ADR 0013](adr/0013-desktop-configuration-is-json-derived-from-the-aggregate.md) |

## Checks

Every check below lives in the desktop's `security` module (`desktop/src/security/`), and nowhere
else. A check is a pure function that takes the facts it needs and returns an allowance or a refusal
with a reason. A new check is added to the module and to this table in the same change.

| Check | Enforces | Status |
|---|---|---|
| Tool protocol input | Every frame is decoded at the tool host's entry. A frame over 1 MiB is rejected before it is read, JSON keeps the decoder's default nesting limit, and malformed input returns an error, never a panic. | Implemented: the limit by `massive_protocol::frames`, the decoding in the tool host (`desktop/src/tools/tool_host/connection.rs`) |
| Connection target | A tool connection is bound to the one MCP root that covers its target, or declined. A target outside every MCP root is declined like one that does not exist. A call is carried out only while its binding lives, so a call queued before its MCP root changed fails. | Implemented: `security::connecting_to_target`, the bindings in the tool host |
| Coverage | Every slot, launcher and instance a tool call names lies inside its MCP root's subtree, as source and as destination. An instance outside is reported as unknown, and no reply names a project above the MCP root, not even through its target path. | Implemented: `security::naming_target` for projects, slots, launchers and instances, carried out in `desktop/src/tools/desktop_tools.rs`. The replies name nothing above the MCP root. |
| Slot bounds | A slot a tool call names lies at most one column right of and one row below its matrix's assigned slots. The desktop lays out every column and row up to a slot, so a far slot would make it lay out a vast empty matrix. | Implemented: `security::naming_slot` |
| Access scope | Every tool call needs an access scope the MCP root grants. Creating a launcher with params also needs `act`, because they decide what the user runs by starting an instance. A removal never stops running instances, so an agent stops them first, which needs `act`. Only the CLI's `remove` asks the planner to stop them. | Implemented: `security::calling_tool` for the tool's scope, `security::creating_launcher` for params, and the planner's refusal of removals while instances run |
| No MCP configuration from inside | `ConfigureMcp` is refused when it comes from an instance that any MCP root covers. | Implemented: `security::configuring_mcp` |
| No nested MCP roots | No change may place an MCP root inside another MCP root's subtree, and the root project is never an MCP root. A loaded file's nested roots lose their `mcp` object. | Enabling is checked by `security::making_mcp_root`. A tool's move stays inside one MCP root, which holds no other, so it cannot nest one. No keyboard or CLI command moves content between projects. |
| Nesting depth | No change nests a project more than 16 levels below the root project, or a launcher's params more than 4 levels, so every configuration the desktop saves loads again. These limits hold for every issuer, so they are desktop policy, not a check of this module. | Implemented: `when::nesting_projects` in the configuration planner for every command, `when::nesting_params` when a tool's params are decoded, and both when a configuration loads. A file past a limit fails to load with an error that names the depth. |
| No desktop-wide changes | A tool connection cannot change the focus, the camera or the startup launcher. | Implemented: every tool command that would move the focus carries `FocusIntent::Keep`, and an instance a tool starts takes no position or animation from the user's focus. No tool sets the startup launcher. |
| Tool ownership | A call to an application tool targets an instance whose application declared that tool. | Planned (ADR 0036) |

## Known gaps

These are the known ways around the checks. Each one is accepted for now, and each one stays listed
until it is closed.

1. **An agent that runs commands outside every MCP root can change access scopes.** It can run the
   CLI's `mcp enable` there. The check above only stops processes inside an MCP root.
2. **Any same-user process can edit `~/.massive/desktop.json`.** The edited access scopes apply at
   the next desktop start. The project header shows them, but nothing announces them until the
   desktop has transient notifications.
3. **Any same-user process can open the desktop socket and connect to any MCP root.** It can pass
   `--target` with a path inside another MCP root. Binding a connection to one MCP root keeps an agent's
   view focused, but it does not isolate MCP roots from each other.
4. **An agent with a shell does not need MCP at all.** It can run commands and read files on its own,
   with the user's full rights.
5. **`act` can be arbitrary command execution.** An application whose tools send input to a
   program that runs commands runs whatever it is sent, with the user's rights. Granting `act` on an
   MCP root grants that for every such instance the root covers. Applications list their own
   security-relevant behavior in their own documentation.

6. **Nothing limits how much an agent stores.** An agent with `organize` can create any number of
   launchers and projects, and params of up to 1 MiB per call with `act`. Every call rewrites the
   whole configuration file on the desktop loop, so a loop of calls grows the file and slows the
   desktop down. Names are limited to 1000 characters.
Access scopes are therefore a hard limit for agents whose only reach is MCP, and a statement of the
user's intent for agents that also have a shell. They are not a sandbox.

## Review checklist

A change that touches a surface above answers these questions in its description:

- Does it add a way for a process to act on the desktop? Then it names the checks that guard it.
- Does it add or change a check? Then the check lives in the `security` module and is listed above.
- Does it open or close a known gap? Then the list above changes with it.
