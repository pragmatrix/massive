//! The desktop's MCP tools (ADR 0036). The tool host serves tool connections on the desktop socket,
//! and the desktop tools bind each connection to its MCP root and plan its calls. The desktop loop
//! reaches the feature through [`ToolHost`], and the desktop system only plans and carries out the
//! commands a tool issues.

mod desktop_tools;
mod tool_host;

pub use desktop_tools::{DesktopTools, ToolCallPlan};
pub use tool_host::{IssuedCommand, ToolHost, ToolRequest};
