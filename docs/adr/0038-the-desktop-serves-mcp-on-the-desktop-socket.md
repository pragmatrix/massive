# The desktop serves MCP on the desktop socket

## Status: proposed

Replaces the tool protocol of
[ADR 0036](./0036-an-mcp-root-exposes-a-project-subtree-to-agents.md). It takes up the option
"The desktop speaks MCP directly, and the bridge only copies bytes", which ADR 0036 rejected.
Everything else in ADR 0036 stays: MCP roots, access scopes, the `security` module, the tool
catalog, the typed tool API, forwarded calls, and the MCP bridge as the agent's stdio MCP server.
Security-relevant consequences are tracked in [the security document](../security.md).

## Problem

The tool protocol of ADR 0036 mirrors MCP message by message. The MCP bridge translates MCP's
`tools/list` into the tool list of `Connected`. It translates `tools/call` into `CallTool` and
`ToolResult`, and `notifications/cancelled` into `CancelTool`. Call ids, cancellation, tool
descriptions and error results therefore exist twice, once in MCP and once in `tools::v1`. Each
side of the translation has its own code and its own tests.

Measured on massive's master at `bd0a45ca`, the tool protocol and its two endpoints take about 830
lines without tests:

| File | Lines without tests | Role |
|---|---|---|
| `protocol/src/tools/v1.rs` | 85 | The frozen messages of the tool protocol. |
| `protocol/src/frame.rs`, `protocol/src/connect.rs` | 67 | The frame codec and the `Connect` frame. |
| `mcp/src/connection.rs` | 282 | The bridge's side of a tool connection. |
| `mcp/src/bridge.rs` | 207 | The bridge's MCP server, which translates between MCP and the tool protocol. |
| `desktop/src/tools/tool_host/connection.rs` | 192 | The desktop's side of a tool connection. |

Their tests add about 330 lines, including the golden bytes that freeze `tools::v1`.

ADR 0036 rejected serving MCP from the desktop for two reasons. It would put `rmcp` into the
desktop, and the desktop would speak a protocol whose versions it does not control. Neither reason
holds as strongly as it seemed:

- The desktop already runs on tokio's workers (`shell/Cargo.toml`), which is the runtime rmcp
  needs. rmcp therefore adds a dependency, but no runtime.
- The bridge already speaks MCP through rmcp, so the MCP versions that agents see already follow
  rmcp's updates. The tool protocol adds a second version that has to stay in step with the first.
- rmcp 3.7.0, the version mt locks, serves any async byte stream with its `transport-async-rw`
  feature, and `tokio::net::UnixStream` is one. Its `AsyncRwTransport::with_max_line_length`
  bounds every incoming message, as the frame limit does today.

## Decision

**The desktop speaks MCP on every tool connection.** The tool host runs one rmcp server per tool
connection over the connection's `UnixStream`. Every message after the `Connect` frame is MCP, in
MCP's stdio framing of one JSON-RPC message per line. The tool protocol and the `tools::v1` module
of `massive-protocol` are removed.

**The `Connect` frame stays, and a new variant names MCP.** The first frame on the desktop socket is
still the postcard encoding of the `Connect` enum, so the out-of-process applications plan can
still add its protocol as another variant. `Connect::Mcp { target }` is added after
`Connect::Tools`, and the desktop closes every connection that sends `Tools`. Postcard encodes a
variant by its index, so keeping `Tools` in its place means that an older bridge is refused
instead of misread. `Mcp` carries no version, because MCP negotiates its own version during
`initialize`. The desktop sends no reply frame.

**The tool host answers `initialize` according to the binding.** The desktop binds the connection
to the MCP root that covers the target before MCP starts, as ADR 0036 decides. A bound connection
offers the `tools` capability and lists the tools that the MCP root's access scopes grant. A
declined connection answers `initialize` without the `tools` capability, and its `instructions`
give the reason, such as "this process is not inside an MCP root". ADR 0036 gave this behavior to
the bridge, and it moves into the desktop.

**The tool host implements rmcp's `ServerHandler` by hand.** It overrides `list_tools` and
`call_tool`, and it looks every tool name up in the tool catalog, as it does today. It uses neither
rmcp's `ToolRouter` nor its `#[tool]` macros, because application tools are declared at runtime by
the applications in the application registry. The typed tool API in `massive-applications` stays
the only way to declare a tool, and `massive-applications` does not depend on rmcp. The tool
catalog turns each declaration into an rmcp `Tool`.

**MCP carries call ids and cancellation.** MCP's `notifications/cancelled` arrives as the
cancellation token of the call's request context. The tool host turns it into the same cancel that
`CancelTool` caused before. The desktop still chooses its own ids for forwarded calls, because the
request ids of different tool connections may collide.

**Tool errors keep their form.** A failed call is still a tool result with MCP's `isError` set and
the text `<code>: <sentence>`. The codes keep the meanings that
[`docs/mcp-desktop-tools.md`](../mcp-desktop-tools.md) lists. A result over 1 MiB still fails the
call with `too_large`.

**A changed MCP root still ends its tool connections.** When a rule of ADR 0036 ends a tool
connection, the tool host answers every pending call with the tool error `connection_closed` and
then closes the socket. MCP's `notifications/tools/list_changed` could keep a connection alive
across a change of access scopes, but nothing needs that yet.

**The MCP bridge copies bytes.** `mcp serve` connects to the desktop socket and writes the
`Connect` frame. It then copies stdin to the socket and the socket to stdout until either side
ends. It never parses MCP while a desktop serves it. The bridge serves a small rmcp server of its
own only when no desktop can serve it. That happens when it cannot reach the desktop socket, or
when it has neither `MASSIVE_INSTANCE` nor `--target`. This server offers no tools and gives the
reason in its `instructions`, so a user can still register the bridge once in their agent's user
configuration. `massive-mcp` keeps rmcp only for this case.

**The tool host stays the security boundary, and rmcp's decoding now runs inside it.** The tool
host bounds every incoming line to 1 MiB with `with_max_line_length`, and it keeps `serde_json`'s
default nesting limit. Malformed input must still end in an error and never in a panic, because
massive's release profile aborts on a panic. A fuzz test of the input therefore drives rmcp's
transport together with the tool host's handler, instead of a frame decoder.

## Consequences

- `massive-desktop` depends on `rmcp` with the `server` and `transport-async-rw` features. The
  desktop's MCP version follows rmcp's updates, as the bridge's already does.
- `massive-protocol` shrinks to the desktop socket's path, the frame codec, and the `Connect` enum.
  The `tools::v1` module and its golden bytes are removed.
- `massive-mcp` shrinks to the byte copy and the server that offers no tools.
- The tool host's binding, its forwarded calls, their cancellation and the security checks stay
  as they are. Only the task that serves a connection changes, from the frame codec to an rmcp
  `ServerHandler`.
- A request over 1 MiB gets no answer. rmcp drops the line before it can read the request's id, so
  the agent's client fails the call on its own timeout instead of receiving `too_large`. The
  connection stays open.
- Tests drive the tool host with an rmcp client over `tokio::net::UnixStream::pair`, so they speak
  the same MCP that agents speak.
- The estimated saving is about 550 lines of code and the tests of `tools::v1`. The larger gain is
  one protocol on a tool connection instead of two.
- When this is implemented, the glossary changes. A tool connection speaks MCP, the entry for the
  tool protocol is removed, and the MCP bridge is no longer the only process that speaks MCP. The
  security document records rmcp's decoding as part of the tool host, and the changed answer to an
  oversized request.

## Considered options

- **Keep the tool protocol of ADR 0036.** Rejected. It mirrors MCP message by message. Every MCP
  feature a tool needs later, such as progress notifications or a changed tool list, would need
  another tool protocol message and another translation in the bridge.
- **Declare the desktop's tools with rmcp's `#[tool_router]` macros.** Rejected. Application tools
  are declared at runtime, so the tool catalog would need a second way to declare a tool, and
  `massive-applications` would depend on rmcp.
- **A bridge that exits when no desktop serves it.** Rejected. An agent then reports a failed MCP
  server in every session outside the desktop, so the bridge could not be registered once for all
  sessions.
- **Reuse index 0 for `Connect::Mcp`.** Rejected. An older bridge's `Connect::Tools` frame would
  then decode as `Mcp` with a garbled target, instead of being refused.
