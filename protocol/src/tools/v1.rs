//! Version 1 of the tool protocol. This module is frozen: it is never edited after its release.
//!
//! A tool connection runs like this. The client sends [`crate::Connect::Tools`], and the desktop
//! answers with one [`ConnectReply`]. After [`ConnectReply::Connected`], the client sends
//! [`ClientMessage`]s and the desktop sends [`HostMessage`]s, in any order and with several calls
//! in flight. The desktop ends the connection with [`HostMessage::Closing`].

use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;

/// The desktop's answer to [`crate::Connect::Tools`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectReply {
    /// The desktop does not speak the requested version. This variant keeps its index and shape in
    /// every version of the tool protocol, so a client of any version can read it.
    Unsupported { supported_versions: Vec<u16> },
    /// The connection is bound to the MCP root that covers the target. The reply names nothing but
    /// the tools, so a connection learns nothing about what lies outside its MCP root, not even the
    /// names of the projects above it.
    Connected {
        /// Every tool the MCP root's access scopes grant, the desktop's and the applications' alike.
        tools: Vec<ToolDescription>,
    },
    /// No MCP root covers the target, or the target does not exist. The reason does not say which.
    Declined { reason: String },
}

/// A tool, as MCP's tool list describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescription {
    /// The tool's name, such as `desktop_describe`. Clients look tools up by it and never split it.
    pub name: String,
    pub description: String,
    /// The JSON Schema of the tool's arguments, as JSON text.
    pub input_schema: String,
    /// The JSON Schema of the tool's result, as JSON text.
    pub output_schema: String,
}

/// A message from the tool client to the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Calls `tool` with `arguments`, a JSON object as text. The client chooses `call_id`, and the
    /// desktop answers with a [`HostMessage::ToolResult`] that carries it.
    CallTool {
        call_id: u64,
        tool: String,
        arguments: String,
    },
    /// The agent cancelled the call. The desktop may still answer it.
    CancelTool { call_id: u64 },
}

/// A message from the desktop to the tool client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostMessage {
    ToolResult {
        call_id: u64,
        outcome: ToolOutcome,
    },
    /// The desktop ends the connection after this message, for example because the MCP root's
    /// access scopes changed. The client fails every call still pending.
    Closing {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolOutcome {
    /// The tool's result, a JSON value as text.
    Ok(String),
    /// The tool failed. `code` names the failure, such as `unknown_id`, and `message` is a sentence
    /// for the agent.
    Err { code: String, message: String },
}

/// The error code of a call whose tool connection ended before the desktop answered it, for
/// example because the MCP root's configuration changed.
pub const CONNECTION_CLOSED: &str = "connection_closed";

/// The error code of a call whose arguments or result do not fit into one frame (see
/// [`crate::MAX_FRAME_LENGTH`]). Only that call fails, and the connection stays open.
pub const TOO_LARGE: &str = "too_large";

/// The wire format of version 1 is frozen. These bytes change only if an edit to this module broke
/// it.
#[cfg(test)]
mod golden {
    use super::*;
    use crate::{Connect, Target, encode};

    fn assert_bytes(message: &impl Serialize, expected: &[u8]) {
        assert_eq!(encode(message).unwrap(), expected);
    }

    #[test]
    fn version_1_messages_keep_their_bytes() {
        assert_bytes(
            &Connect::Tools {
                version: VERSION,
                target: Target::Path("/a".into()),
            },
            &[0, 1, 1, 2, 47, 97],
        );
        assert_bytes(
            &Connect::Tools {
                version: VERSION,
                target: Target::Instance("i".into()),
            },
            &[0, 1, 0, 1, 105],
        );
        assert_bytes(
            &ConnectReply::Unsupported {
                supported_versions: vec![1, 2],
            },
            &[0, 2, 1, 2],
        );
        assert_bytes(
            &ConnectReply::Connected {
                tools: vec![ToolDescription {
                    name: "t".into(),
                    description: "d".into(),
                    input_schema: "{}".into(),
                    output_schema: "{}".into(),
                }],
            },
            &[1, 1, 1, 116, 1, 100, 2, 123, 125, 2, 123, 125],
        );
        assert_bytes(&ConnectReply::Declined { reason: "x".into() }, &[2, 1, 120]);
        assert_bytes(
            &ClientMessage::CallTool {
                call_id: 300,
                tool: "t".into(),
                arguments: "{}".into(),
            },
            &[0, 172, 2, 1, 116, 2, 123, 125],
        );
        assert_bytes(&ClientMessage::CancelTool { call_id: 7 }, &[1, 7]);
        assert_bytes(
            &HostMessage::ToolResult {
                call_id: 7,
                outcome: ToolOutcome::Ok("{}".into()),
            },
            &[0, 7, 0, 2, 123, 125],
        );
        assert_bytes(
            &HostMessage::ToolResult {
                call_id: 7,
                outcome: ToolOutcome::Err {
                    code: "c".into(),
                    message: "m".into(),
                },
            },
            &[0, 7, 1, 1, 99, 1, 109],
        );
        assert_bytes(&HostMessage::Closing { reason: "x".into() }, &[1, 1, 120]);
    }
}
