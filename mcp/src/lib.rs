//! The MCP bridge: the only process that speaks MCP (ADR 0036).
//!
//! An application offers the bridge as its `mcp serve` subcommand, and an agent runs that as a
//! stdio MCP server. The bridge is a tool client. It connects to the desktop socket, names its
//! target, and serves the tools of the MCP root that covers the target. It knows no tool: it turns
//! MCP's tool calls into tool protocol messages and their results back into MCP results.

mod bridge;
mod connection;

pub use bridge::serve;
