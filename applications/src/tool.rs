//! The typed tool API through which the desktop and applications declare their MCP tools
//! (ADR 0036). A tool is a parameter type and a result type, both deriving serde's traits and
//! `schemars`' `JsonSchema`. The API derives the JSON Schemas from the types, decodes the arguments,
//! and encodes the result, so a provider never handles JSON itself.

use std::fmt;

use derive_more::Into;
use schemars::{JsonSchema, Schema, schema_for};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::AccessScope;

/// A declared tool, as MCP's tool list shows it.
#[derive(Debug, Clone)]
pub struct ToolDeclaration {
    pub name: ToolName,
    pub access_scope: AccessScope,
    pub description: String,
    pub input_schema: Schema,
    pub output_schema: Schema,
}

impl ToolDeclaration {
    /// Declares the tool `name`, which takes `Params` and returns `Output`.
    pub fn new<Params: JsonSchema, Output: JsonSchema>(
        name: ToolName,
        access_scope: AccessScope,
        description: &str,
    ) -> Self {
        Self {
            name,
            access_scope,
            description: description.to_string(),
            input_schema: schema_for!(Params),
            output_schema: schema_for!(Output),
        }
    }
}

/// A tool's name: its provider's MCP id and its own, joined by `_`, such as `desktop_describe`.
/// Names are looked up, never split, because an MCP id may contain `_` itself. [`tool_name!`]
/// checks a name at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToolName {
    provider: McpId,
    tool: McpId,
}

impl ToolName {
    /// The longest joined name. LLM APIs commonly cap tool names at 64 characters, and MCP clients
    /// put a prefix in front, such as Claude Code's `mcp__<server>__`.
    pub const MAX_LEN: usize = 48;

    /// Panics when the joined name is longer than [`Self::MAX_LEN`], at compile time in the const
    /// context [`tool_name!`] provides.
    pub const fn new(provider: McpId, tool: McpId) -> Self {
        assert!(
            provider.0.len() + 1 + tool.0.len() <= Self::MAX_LEN,
            "a tool name joined from its provider's MCP id and its own is at most 48 characters"
        );
        Self { provider, tool }
    }

    /// Whether `name` is this tool name, as a tool call names it.
    pub fn is(self, name: &str) -> bool {
        name.strip_prefix(self.provider.as_str())
            .and_then(|rest| rest.strip_prefix('_'))
            == Some(self.tool.as_str())
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.provider.as_str(), self.tool.as_str())
    }
}

/// A [`ToolName`] of the provider `$provider`, a constant [`McpId`], and the tool `$tool`, checked
/// at compile time.
///
/// ```
/// use massive_applications::{McpId, tool_name};
/// let name = tool_name!(McpId::DESKTOP, "describe");
/// ```
///
/// ```compile_fail
/// use massive_applications::{McpId, tool_name};
/// let name = tool_name!(McpId::DESKTOP, "a_tool_name_so_long_that_it_exceeds_the_limit");
/// ```
#[macro_export]
macro_rules! tool_name {
    ($provider:expr, $tool:literal) => {
        const { $crate::ToolName::new($provider, $crate::McpId::new($tool)) }
    };
}

/// A name of a tool provider or of a tool, matching `[a-z][a-z0-9_-]*` (ADR 0036). These characters
/// pass both MCP's tool name rules and the stricter rules of LLM APIs.
///
/// MCP ids come from code, so an invalid one is a bug of its author. [`mcp_id!`] checks it at
/// compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct McpId(&'static str);

impl McpId {
    /// The desktop's own MCP id, which no application may use.
    pub const DESKTOP: Self = Self::new("desktop");

    /// Panics on an invalid id. In a const context, which [`mcp_id!`] provides, the panic is a
    /// compile error at the call. A const panic message must be a literal, so it names the rule,
    /// not the id.
    pub const fn new(id: &'static str) -> Self {
        let bytes = id.as_bytes();
        assert!(
            !bytes.is_empty() && bytes[0].is_ascii_lowercase(),
            "an MCP id starts with a-z"
        );
        let mut index = 1;
        while index < bytes.len() {
            let byte = bytes[index];
            assert!(
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-',
                "an MCP id contains only a-z, 0-9, _ and -"
            );
            index += 1;
        }
        Self(id)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// An [`McpId`], checked at compile time.
///
/// ```
/// let id = massive_applications::mcp_id!("read_page");
/// ```
///
/// ```compile_fail
/// let id = massive_applications::mcp_id!("ReadPage");
/// ```
#[macro_export]
macro_rules! mcp_id {
    ($id:literal) => {
        const { $crate::McpId::new($id) }
    };
}

/// Decodes a tool call's arguments into the tool's parameter type. serde_json keeps its default
/// nesting limit, so deeply nested arguments fail instead of exhausting the stack.
pub fn decode_arguments<Params: DeserializeOwned>(arguments: &str) -> Result<Params, ToolError> {
    serde_json::from_str(arguments).map_err(|error| ToolError {
        code: ToolErrorCode::InvalidArgument,
        message: format!("The arguments do not match the tool's input schema: {error}."),
    })
}

/// Encodes a tool's result as JSON text.
pub fn encode_result(output: &impl Serialize) -> ToolResultJson {
    ToolResultJson(serde_json::to_string(output).expect("a tool result type serializes to JSON"))
}

/// A tool's result, encoded as JSON text. Tool results travel as text, because the tool protocol's
/// binary encoding cannot hold free-form JSON. Only [`encode_result`] creates one, so the text is
/// always valid JSON.
#[derive(Debug, Clone, PartialEq, Eq, Into)]
pub struct ToolResultJson(String);

/// A failed tool call: an error code and a sentence for the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

/// Why a tool call failed. MCP clients see the code's name, so names never change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCode {
    /// An id names nothing the MCP root covers. It may be stale, or it may lie outside the MCP
    /// root, which the error does not reveal.
    UnknownId,
    ScopeNotGranted,
    InvalidArgument,
    /// The desktop refused the command, for example a move onto an assigned slot.
    Refused,
}

impl ToolErrorCode {
    pub fn name(self) -> &'static str {
        match self {
            Self::UnknownId => "unknown_id",
            Self::ScopeNotGranted => "scope_not_granted",
            Self::InvalidArgument => "invalid_argument",
            Self::Refused => "refused",
        }
    }
}

impl fmt::Display for ToolErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
