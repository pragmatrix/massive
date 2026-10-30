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
| Tool protocol input | Every frame is decoded at the tool host's entry. A frame over 1 MiB is rejected before it is read, JSON keeps the decoder's default nesting limit, and malformed input returns an error, never a panic. | Implemented: the limit by `massive_protocol::frames`, the decoding in the tool host (`desktop/src/tool_host/connection.rs`) |
| Connection target | A tool connection is bound to the one MCP root that covers its target, or declined. A target outside every MCP root is declined like one that does not exist. A call is carried out only while its binding lives, so a call queued before its MCP root changed fails. | Implemented: `security::connecting_to_target`, the bindings in the tool host |
| Coverage | Every slot, launcher and instance a tool call names lies inside its MCP root's subtree, as source and as destination. An instance outside is reported as unknown, and no reply names a project above the MCP root, not even through its target path. | Planned (ADR 0036). The replies name nothing above the MCP root already. |
| Access scope | Every tool call needs an access scope the MCP root grants. A removal that stops instances also needs `act`. | Implemented for the tool's scope: `security::calling_tool`. Removals arrive with the organizing tools. |
| No MCP configuration from inside | `ConfigureMcp` is refused when it comes from an instance that any MCP root covers. | Implemented: `security::configuring_mcp` |
| No nested MCP roots | No change may place an MCP root inside another MCP root's subtree, and the root project is never an MCP root. A loaded file's nested roots lose their `mcp` object. | Enabling is checked by `security::making_mcp_root`. Moves are planned with `desktop_move_slot` (ADR 0036). |
| No desktop-wide changes | A tool connection cannot change the focus, the camera or the startup launcher. | Planned (ADR 0036) |
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

Access scopes are therefore a hard limit for agents whose only reach is MCP, and a statement of the
user's intent for agents that also have a shell. They are not a sandbox.

## Review checklist

A change that touches a surface above answers these questions in its description:

- Does it add a way for a process to act on the desktop? Then it names the checks that guard it.
- Does it add or change a check? Then the check lives in the `security` module and is listed above.
- Does it open or close a known gap? Then the list above changes with it.
