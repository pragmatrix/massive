# An MCP root exposes a project's subtree to agents

## Status: accepted, not implemented

Builds on [ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md) (projects and slots),
[ADR 0024](./0024-configuration-operations-are-one-command-vocabulary.md) (one command vocabulary),
and [ADR 0029](./0029-exceptional-desktop-behaviors-are-decided-by-the-desktop-policy.md) (the
desktop policy). The desktop socket is shared later with
[the out-of-process applications plan](../out-of-process-applications-plan.md).
Security-relevant consequences are tracked in [the security document](../security.md).

## Problem

Coding agents such as Claude Code run inside an instance on the desktop. They work on a project,
and they would gain from organizing it themselves. An agent could add a launcher for a dev server,
start it, use the application running in it, and group related instances. Today it cannot do any
of that.

The CLI is the only way a process reaches the desktop. It writes one-way
`ConfigurationRequest`s into a file descriptor that its instance inherits, and it gets no
answer back. An application cannot answer the desktop either. The desktop sends it events, but the
application has no channel to report a result.

The Model Context Protocol (MCP) is how agents discover and call tools. This ADR decides how the
desktop offers its projects, and the applications running in them, as MCP tools.

## Decision

### An MCP root covers a project's subtree

**A user enables MCP on a project, and that project becomes an MCP root.** An MCP root covers its
whole subtree, which is every slot, launcher, nested project and instance below it. A covered target
is one that lies in that subtree. Everything an MCP root covers is reachable with the same access
scopes. Nested projects do not have settings of their own.

**MCP roots never nest.** An MCP root that lies inside another MCP root's subtree is an invariant
violation, so the desktop rejects every change that would create one. It rejects enabling a project
that has an MCP root above or below it. It also rejects a move, push or group that would place one
MCP root inside another one's subtree.

**The root project is never an MCP root.** It would cover every instance on the desktop, so the
CLI could not disable it again from anywhere, because a process inside an MCP root cannot configure
MCP.

**Ungrouping an MCP root is refused for now.** Ungrouping dissolves the project, and with it the
setting the user made deliberately. The desktop policy (ADR 0029) refuses it with the reason
"disable MCP for `<name>` first". Removing the project stays allowed, because it explicitly deletes
the project and its setting together.

### Access scopes say what a connection may do

**An access scope is a named permission that an MCP root grants.** The name follows OAuth 2's
access token scope (RFC 6749, section 3.3). Every tool declares exactly one access scope. A tool is
offered and callable only when the MCP root grants its scope.

The vocabulary is deliberately broad for now:

| Access scope | Allows |
|---|---|
| `observe` | Describing the covered subtree. |
| `organize` | Every configuration command inside the subtree. |
| `read` | Application tools that only read an instance's content. |
| `act` | Starting and stopping instances, and application tools that change something, such as sending input to an instance. |

Starting an instance runs its launcher's command, which is as powerful as sending input to an
application that runs commands, so both belong to `act`. A removal that would stop running instances needs `act` in addition to
`organize`.

The format leaves room for refinement. A later version may add application-qualified scopes such as
`act:term`, where `act` grants every `act:<mcp id>`. Nothing of that is built now.

**The access scopes are part of the desktop configuration.** The project node in `desktop.json`
stores them as `"mcp": { "access_scopes": ["observe", "act"] }`. The presence of the `mcp` object
makes the project an MCP root, and an empty list grants nothing, so agents see no tools. Every
scope, `observe` included, is granted explicitly. In the desktop, an MCP root carries an
`McpConfiguration` that holds its access scopes, and every other project carries none. Changing
it is a new configuration command, `ConfigurationCommand::ConfigureMcp { project, mcp }`, whose
`mcp` is `None` to disable MCP. Enabling and disabling MCP and changing the scopes are all the
same command. The CLI issues it as `mcp enable [PATH] [--access-scopes observe,act]` and
`mcp disable [PATH]`. MCP itself has no way to issue it.

**A process inside an MCP root cannot configure MCP.** The desktop refuses
`ConfigureMcp` whenever it comes from an instance that any MCP root covers. That includes
widening, narrowing and disabling, and it applies to every MCP root, not only to the one covering
the instance. The desktop knows which instance a CLI request comes from by the file descriptor that
instance inherited, so a process cannot claim to be elsewhere by changing its environment. The
keyboard and processes outside every MCP root are unaffected.

### Agents connect through the MCP bridge

**The MCP bridge is the only process that speaks MCP.** An application offers it as the
`mcp serve` subcommand of its binary, and an agent configures that as a stdio MCP server. The
bridge's code lives in a massive crate, `massive-mcp`, built on `rmcp`, the official Rust MCP SDK.
Any massive-based application can offer it as a subcommand. The desktop does not depend on
`massive-mcp` or on MCP.

**The bridge is generic.** It turns MCP's tool calls into tool protocol messages and their results
back into MCP results. It knows no tool and no access scope, and never looks inside a tool's
arguments or results. It passes error codes on as names, without interpreting them.

**The bridge is no security boundary.** Any same-user process can open the desktop socket and
speak the tool protocol without going through the bridge. The boundary is the desktop's tool host,
described below.

**The bridge is a tool client, and it reaches the desktop through a tool connection.** A tool
client is any process that connects to the desktop to call tools. A tool connection is its Unix
socket connection to the desktop socket, which speaks the tool protocol described below. The
desktop socket is `~/.massive/run/desktop.sock`, in a directory the desktop creates with mode
`0700`. `$TMPDIR` is not used, because macOS purges files there that go unused for a few days, and
the desktop runs for long periods.

**The desktop socket exists while the desktop runs.** Only the user can reach it, because its
directory has mode `0700`, and the user is the trust boundary (see the security document). Without
an MCP root, every tool connection is declined, so no tool is reachable. A missing socket therefore
always means that no desktop runs.

**The tool host learns about changed MCP roots from the changes that change them.** Only
`ConfigureMcp` and the removal of a project that is an MCP root configure, reconfigure, disable or
remove one. They say so in the transaction's change surface, and the tool host closes the tool
connections bound to those MCP roots.

**The desktop tells every process it starts where to connect.** Every application learns the
desktop socket's path and the id of its instance from its instance context. It passes both to the
processes it starts, as `MASSIVE_SOCKET` and `MASSIVE_INSTANCE`.

### A tool connection is bound to the MCP root that covers its target

**A tool client connects to a target.** The target is a desktop target, either an instance, named
by its id from `MASSIVE_INSTANCE`, or a target path from `mcp serve --target PATH`. The target path
may name a project or a launcher. The desktop walks up from the target's project to the MCP root at
or above it. Because MCP roots never nest, at most one exists, so the binding involves no choice.

The reply names only the tools, every tool the MCP root's access scopes grant, the desktop's and
the applications' alike, in the form of MCP's tool list. The bridge answers MCP's `tools/list` from them without asking the desktop again.
Otherwise the reply declines with a reason. An instance or a target path that no MCP root covers is
declined the same way as one that does not exist, so a connection learns nothing about what lies
outside every MCP root.

**A bridge that no MCP root covers still starts.** This happens when the desktop is not running,
when the target lies outside every MCP root, or when there is neither `MASSIVE_INSTANCE` nor
`--target`. The bridge then answers MCP's `initialize` without the `tools` capability, which is
MCP's way to say that a server offers no tools. The reply's `instructions` field explains why, for
example "this process is not inside an MCP project". It does not name other MCP roots. A user can
therefore register the bridge once in their agent's user configuration, and it stays harmless in
every instance no MCP root covers.

**A tool connection lives as long as its MCP root stays unchanged.** Removing the MCP root,
changing its access scopes, or quitting the desktop ends the connection. The desktop first sends
the reason, and the bridge answers every call still pending with the tool error `connection_closed`
and exits. The bridge does not reconnect, and the protocol has no events. A changed MCP root is
picked up by starting a new agent session or reconnecting the MCP server in the agent.

### The tool protocol

**The tool protocol is the only protocol a tool connection speaks.** It has one version for the whole
connection, and its messages live in the frozen module `tools::v1` of the crate `massive-protocol`,
which the desktop and `massive-mcp` share. There is no separate session layer. The first frame on
the desktop socket is a `Connect` enum whose variant names the protocol, today only
`Connect::Tools { version, target }`. A later protocol, such as the one of the out-of-process
applications plan, adds a variant at the end. Postcard encodes a variant by its index, so an older
desktop cannot decode a newer variant and closes the connection.

**Frames are length-prefixed postcard.** Every frame is a 4-byte big-endian length followed by the
postcard encoding of one message. A frame may be at most 1 MiB in either direction, which is the
protocol's only size limit. A tool's arguments and its result travel as JSON text inside the
messages, so tools can change without a new version of the protocol. Error codes travel as names, so a
new error code needs no new version either. Access scopes never travel. The desktop checks them,
and the tool list contains only the tools they grant.

| Message | Direction | Meaning |
|---|---|---|
| `Connect::Tools { version, target }` | client to desktop, first | Names the protocol version and the target. |
| `Connected { tools }` | desktop to client, reply | The connection is bound to the MCP root that covers the target. `tools` are the tools its access scopes grant. |
| `Declined { reason }` | desktop to client, reply | No MCP root covers the target. |
| `Unsupported { supported_versions }` | desktop to client, reply | The desktop does not speak the requested version. |
| `CallTool { call_id, tool, arguments }` | client to desktop | Calls a tool by its name. The client chooses the call id. |
| `CancelTool { call_id }` | client to desktop | The agent cancelled the call. |
| `ToolResult { call_id, outcome }` | desktop to client | The tool's result as JSON text, or an error code and a sentence. |
| `Closing { reason }` | desktop to client, last | The desktop ends the connection, for example because the MCP root's access scopes changed. |

Several calls may be in flight on one connection. Ids inside arguments and results are session ids.
They are valid only while the desktop runs, and a call that names a stale id fails.

**The tool host is the security boundary.** The tool host is the part of the desktop that serves
tool connections. It decodes every frame at its entry, before anything else runs. It rejects every
frame over 1 MiB before reading it, keeps the JSON decoder's default nesting limit, and answers
malformed input with an error, never with a panic. The nesting limit matters even below the size
limit, because 1 MiB of nested arrays would otherwise exhaust the stack of a recursive decoder.
massive's release profile aborts on a panic, so a panic in the tool host would end the desktop with
every instance.

**The desktop checks every call.** Every call needs an access scope the MCP root grants. For a
desktop tool, every slot, launcher and instance the decoded call names, as source and as
destination, must lie inside the MCP root's subtree. For an application tool, the instance must lie
inside the subtree, and its application must provide the tool. A call cannot change the focus or
the camera on its own, and it cannot set the startup launcher, which belongs to the whole desktop.
**A tool connection learns nothing about what lies outside its MCP root.** That covers instances,
which a call reports as unknown when they lie outside, and the names of the projects above the MCP
root, which no reply contains. A connection therefore never sees the MCP root's target path, only
its name.

Several tool connections may be open on one MCP root. Their calls go through the desktop's
event loop in order, together with keyboard and CLI commands.

### Tools are named by MCP ids

**An MCP id names a tool provider or a tool.** A tool provider is the desktop, or an application
with an MCP id. `McpId` is a validated type in `massive-applications`. Constructing one from a
string that does not match `[a-z][a-z0-9_-]*` is an error, so an invalid MCP id cannot exist. These
characters pass both MCP's tool name rules and the stricter rules of LLM APIs, which reject dots.

**An application opts into MCP by giving itself an MCP id.** It calls
`Application::with_mcp_id(McpId)`. An application without an MCP id offers no tools, but
`desktop_describe` still lists its instances. The desktop's own MCP
id is `desktop`, which no application may use.

**A tool's name joins the provider's MCP id and the tool's MCP id with `_`.** An application with the MCP id
`notes` and a tool `read_page` offers `notes_read_page`, for example. The provider of an MCP id decides
whether it uses `_` or `-` inside it. That makes a name unparseable in general, so the bridge and
the desktop never split names. They look names up in the tool list instead. The application registry
rejects a duplicate MCP id and any two tools whose joined names collide.

### Tools are declared as Rust types

**The desktop and every application declare their tools through one typed tool API in
`massive-applications`.** A tool is a parameter type and a result type, both deriving serde's
traits and `schemars`' `JsonSchema`, together with the tool's MCP id, its access scope and a
description. A declaration reads like
`Tool::new::<ReadPage, ReadPageResult>("read_page", AccessScope::Read, "…")`. A provider's tools are
fixed for the life of the desktop, so they cannot change during a tool connection.

**The tool API does all the JSON work.** It derives each tool's input and output schemas from the
types, so a schema cannot drift from the code. It decodes the arguments into the parameter type
within the tool host's limits. It encodes the result as MCP's `structuredContent` and repeats the
same JSON as text, as the MCP specification recommends. A provider fails a call with a typed error
code, which the API turns into a tool result with MCP's `isError` set and the text
`<code>: <sentence>`. A provider never handles JSON itself.

**Each provider decodes its own tools' arguments.** The desktop decodes the arguments of its own
tools in the tool host. For an application tool, the tool host reads only `instance_id`,
removes it from the arguments, checks the call, and forwards the rest. The application side of the
tool API decodes them inside the instance, today in the desktop's process and later in the
out-of-process client library. Every argument is therefore decoded once, by the code that owns it.

### The desktop's tools

The desktop's tools are fixed. `desktop_describe` returns the covered subtree. One tool covers
each `ConfigurationCommand` of ADR 0024, except setting the startup launcher, and two tools start
and stop instances. [`docs/mcp-desktop-tools.md`](../mcp-desktop-tools.md) specifies every tool
with its parameters, results and errors.

**A command issued through MCP never moves the user's focus.** When the keyboard issues a
configuration command, the command decides where the focus goes afterwards (ADR 0024). Commands
therefore carry a focus intent. The keyboard and the CLI issue `Follow`, which keeps today's
behavior. MCP issues `Keep`, and the planner then leaves the keyboard focus and the camera where
they are. Only a focused target that the command removes still moves the focus, because the focus
cannot stay on something that no longer exists. The selection survives every change that leaves
the slots of its own project alone, so an agent working elsewhere never takes it away. An agent can
therefore reorganize a project next to the user without pulling the user's view away.

### Applications answer calls

**A tool call reaches the application as a message, and its result returns as an instance
change.** The desktop delivers `ApplicationMessage::ToolCall { call_id, tool, arguments }` to the
instance. That message is never coalesced and stays ordered with the instance's input events. The
tool API decodes it before the application sees it. The application answers through the tool API,
which submits `InstanceChange::ToolResult { call_id, outcome }` immediately.

**A tool call may take as long as it needs.** The desktop sets no deadline. A call ends when the
application answers, when the agent cancels it, or when the tool connection closes. The desktop
sends `ApplicationMessage::CancelToolCall { call_id }` in the last two cases. A tool that waits takes
its own `timeout_ms` argument, and the application enforces a maximum. An application keeps a
waiting call pending without blocking its event loop, and several calls may be in flight per
instance. Detecting an application that hangs belongs to the out-of-process applications plan.

### MCP roots are visible

**The MCP access indicator names an MCP root's access scopes in its header.** It reads
`home · MCP observe organize`, or `home · MCP` when the MCP root grants nothing.

**The MCP activation indicator colors everything an MCP root covers.** The MCP root's header, the
headers of the projects below it, its launchers, and the title bars of its instances take their
colors from a `policy::colors::mcp` module. That module has a counterpart for every color it replaces, so primary
and assistant title bars stay distinguishable inside an MCP root.

### Security checks live in one module

**Every security-relevant check of the desktop lives in the `security` module
(`desktop/src/security/`).** This covers bounding tool protocol input, binding a tool connection to the MCP root of its target,
checking that a call's targets are covered, checking access scopes, refusing `ConfigureMcp`
from a covered instance, and rejecting nested MCP roots. Each check is a pure function that takes
the facts it needs and returns an allowance or a refusal with a reason, like the rules of the
desktop policy (ADR 0029). The code that handles a call gathers the facts, asks the `security`
module, and carries out the answer.

The `security` module is not the desktop policy. A policy rule decides a behavior that could
reasonably have been decided differently. A security check enforces a boundary that holds
regardless of taste. The ungrouping refusal above is a policy rule, because it protects a setting
rather than a boundary.

[`docs/security.md`](../security.md) lists every check of the `security` module, the trust model,
and the known ways around it.

## Consequences

- The desktop gains a socket and a second way to issue configuration commands. ADR 0024's rule
  holds for it. A desktop tool names slots by ids and positions and issues a
  `ConfigurationCommand`, and it never plans changes itself.
- Applications gain a reply path to the desktop for the first time. It carries only tool results.
- `massive-applications` gains the typed tool API and with it `serde_json` and `schemars`.
  `Application` gains `with_mcp_id` and tool declarations. Applications without them are
  unaffected.
- `massive-mcp` stays small, because it knows no tool. Adding a tool touches only its provider.
- `desktop.json` gains the optional `mcp` object on project nodes.
- Configuration commands and `StartInstance` gain a focus intent, so the planner can carry out a
  command without moving the focus.
- The desktop socket in `~/.massive/run/` and `massive-protocol` are built before the
  out-of-process applications plan, which adds its own protocol as another `Connect` variant.
- Applications gain `MASSIVE_SOCKET` and `MASSIVE_INSTANCE` from their instance context, and pass
  them to the processes they start.
- **Access scopes are not a sandbox.** They are a hard limit for agents whose only reach is MCP. For
  agents that also have a shell they state the user's intent, and the known ways around them are
  listed in [`docs/security.md`](../security.md).

## Considered options

- **HTTP on localhost instead of a Unix socket and a stdio bridge.** Rejected. Every local process
  and every browser tab could reach the port, which would need tokens and DNS-rebinding guards.
  Every MCP client supports stdio.
- **The desktop speaks MCP directly, and the bridge only copies bytes.** Rejected. It puts `rmcp`
  into the desktop, and the desktop would speak a protocol whose versions it does not control.
- **A tool protocol message per desktop tool.** Rejected. Each desktop tool lived in three
  places, as a schema in the bridge, a tool protocol message, and a handler in the desktop. The bridge
  parsed values that the desktop had to check again anyway, because any process can skip the
  bridge.
- **Each provider handles JSON itself.** Rejected. Every application would repeat decoding,
  schemas, input limits and error formatting, and each copy could differ.
- **Applications run their own MCP servers.** Rejected. Agents would juggle one connection per
  application, and every application would have to enforce coverage and access scopes itself.
- **Tools registered per instance at runtime.** Rejected. The tool list would change with every
  instance, and agents would carry a growing list in their context.
- **Ordered access levels (`read` < `organize` < `full`).** Rejected in favor of access scopes,
  which can be refined per application later without reordering anything.
- **A separate `launch` scope for starting and stopping instances.** Rejected. Starting a launcher
  runs its command, which is as powerful as `act`.
- **Inherited settings, where nested projects override their parent.** Rejected. One MCP root with
  one set of access scopes is easier to see and to reason about.
- **One connection that reaches every MCP root.** Rejected. An agent would have to pick a root for
  every call.
- **A separate bind message after the handshake.** Rejected. Because MCP roots never nest, the
  target determines the MCP root without a choice, so it belongs in the first message.
- **A session layer shared by every protocol, with roles, per-protocol versions, a byte order and a
  role payload.** Rejected for now. It would carry fields that only the out-of-process
  applications need, and its role payload would be decoded twice. A `Connect` variant per protocol
  identifies the protocol with one decode.
- **WebSocket framing.** Rejected. It adds an HTTP upgrade, masks every byte the client sends, and
  brings features the tool protocol does not need. A length prefix gives the same message
  boundaries.
- **A desktop socket that exists only while an MCP root exists.** Rejected. Without an MCP root,
  no tool is reachable anyway, and only the user can reach the socket. The rule would cost a socket
  lifecycle, and a missing socket would not tell whether a desktop runs.
- **One socket per protocol.** Rejected. `MASSIVE_SOCKET` would name only one of them, and each
  socket would need its own lifecycle.

## Follow-ups

- Show the enabled MCP roots and their access scopes at startup, once the desktop has transient
  notifications ([`docs/desktop-notifications-plan.md`](../desktop-notifications-plan.md)).
- Enabling MCP and changing access scopes from the keyboard.
- Application-qualified access scopes such as `act:term`.
