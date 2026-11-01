//! The desktop's MCP tools (ADR 0036).
//!
//! The tool host serves tool connections on the desktop socket, and the desktop tools bind each
//! connection to its MCP root and check its calls. The catalog lists the desktop's tools and the
//! applications' tools. The tool host forwards a call of an application's tool to its instance.
//! The desktop loop reaches the feature through [`ToolHost`], and the desktop system only plans
//! and carries out the commands a tool issues.

mod desktop_tools;
mod tool_catalog;
mod tool_host;

pub use desktop_tools::{DesktopTools, InstanceCall, ToolCallPlan};
pub use tool_catalog::{ApplicationTool, ToolCatalog};
pub use tool_host::{IssuedCommand, ToolHost, ToolRequest};
