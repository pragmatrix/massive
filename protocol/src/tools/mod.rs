//! The tool protocol: the messages a tool client exchanges with the desktop over a tool connection
//! (ADR 0036). It carries tool calls only. A tool's arguments and result travel as JSON text, and
//! error codes travel as names, so neither a new tool nor a new error code needs a new version of
//! the protocol. Access scopes stay inside the desktop, which checks them.

pub mod v1;
