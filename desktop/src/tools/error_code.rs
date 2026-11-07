//! The error codes of the desktop's own failures. [`ToolErrorCode`] explains how codes work, and
//! `docs/mcp-desktop-tools.md` documents these for agents.

use massive_applications::ToolErrorCode;

/// An id names nothing the MCP root covers. It may be stale, or it may lie outside the MCP root,
/// which the error does not reveal.
pub const UNKNOWN_ID: ToolErrorCode = ToolErrorCode::new("unknown_id");

/// The MCP root does not grant the tool's access scope.
pub const SCOPE_NOT_GRANTED: ToolErrorCode = ToolErrorCode::new("scope_not_granted");

/// The desktop refused the command, for example a move onto an assigned slot of another project.
pub const REFUSED: ToolErrorCode = ToolErrorCode::new("refused");
