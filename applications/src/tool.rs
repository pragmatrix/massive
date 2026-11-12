//! The typed tool API through which the desktop and applications declare their MCP tools
//! (ADR 0036). A tool is a parameter type and a result type, both deriving serde's traits and
//! `schemars`' `JsonSchema`. The API derives the JSON Schemas from the types, decodes the arguments,
//! and encodes the result, so a provider never handles JSON itself.

use std::fmt;

use derive_more::From;
use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use massive_protocol::tools::v1::{CONNECTION_CLOSED, TOO_LARGE};

use crate::AccessScope;

/// A tool that the desktop or an application declares. The desktop joins its provider's MCP id and
/// `tool` into the [`ToolName`] that MCP's tool list shows.
#[derive(Debug, Clone)]
pub struct ToolDeclaration {
    /// The tool's own MCP id, such as `describe`.
    pub tool: McpId,
    pub access_scope: AccessScope,
    pub description: String,
    pub input_schema: Schema,
    pub output_schema: Schema,
}

impl ToolDeclaration {
    /// Declares the tool `tool`, which takes `Params` and returns `Output`.
    pub fn new<Params: JsonSchema, Output: JsonSchema>(
        tool: McpId,
        access_scope: AccessScope,
        description: &str,
    ) -> Self {
        Self {
            tool,
            access_scope,
            description: description.to_string(),
            input_schema: tool_schema::<Params>(),
            output_schema: tool_schema::<Output>(),
        }
    }
}

/// Derives the JSON Schema of a tool's parameters or result. MCP clients validate it, and Claude Code
/// warns about every `format` it does not know, such as schemars' `uint32` on integers.
fn tool_schema<T: JsonSchema>() -> Schema {
    SchemaSettings::default()
        .with_transform(RecursiveTransform(remove_integer_format))
        .into_generator()
        .into_root_schema_for::<T>()
}

/// Removes `format` from an integer schema. It only repeats the Rust type, and the schema's
/// `minimum` already keeps unsigned integers non-negative.
fn remove_integer_format(schema: &mut Schema) {
    let is_integer = match schema.get("type") {
        Some(Value::String(name)) => name == "integer",
        Some(Value::Array(names)) => names.iter().any(|name| name == "integer"),
        _ => false,
    };
    if is_integer {
        schema.remove("format");
    }
}

/// A tool's name: its provider's MCP id and its own, joined by `_`, such as `desktop_describe`.
/// Names are looked up, never split, because an MCP id may contain `_` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToolName {
    provider: McpId,
    tool: McpId,
}

impl ToolName {
    /// The longest joined name. LLM APIs commonly cap tool names at 64 characters, and MCP clients
    /// put a prefix in front, such as Claude Code's `mcp__<server>__`.
    pub const MAX_LEN: usize = 48;

    /// Joins the provider's MCP id and the tool's, and refuses a joined name longer than
    /// [`Self::MAX_LEN`].
    pub fn join(provider: McpId, tool: McpId) -> Result<Self, ToolNameTooLong> {
        let name = Self { provider, tool };
        if provider.0.len() + 1 + tool.0.len() > Self::MAX_LEN {
            return Err(ToolNameTooLong(name));
        }
        Ok(name)
    }

    /// The tool's own MCP id.
    pub fn tool(self) -> McpId {
        self.tool
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

/// A joined name that [`ToolName::join`] refused, because it is longer than
/// [`ToolName::MAX_LEN`]. It carries the refused name, so the caller can name it in its error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolNameTooLong(pub ToolName);

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
/// ```compile_fail,E0080
/// let id = massive_applications::mcp_id!("ReadPage");
/// ```
#[macro_export]
macro_rules! mcp_id {
    ($id:literal) => {
        const { $crate::McpId::new($id) }
    };
}

/// A call of one of an application's tools, which the desktop forwards to the instance it names
/// (ADR 0036).
///
/// The desktop has already checked the call, so the instance only matches `tool`, the tool's own
/// MCP id, and decodes the arguments. It answers with [`crate::InstanceContext::answer_tool_call`].
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub tool: McpId,
    arguments: ToolArguments,
}

impl ToolCall {
    /// `arguments` lack the `instance_id` that routed the call, because the desktop removes it
    /// before forwarding.
    pub fn new(id: ToolCallId, tool: McpId, arguments: ToolArguments) -> Self {
        Self {
            id,
            tool,
            arguments,
        }
    }

    pub fn decode<Params: DeserializeOwned>(&self) -> Result<Params, ToolError> {
        decode_arguments(&self.arguments)
    }
}

/// What the desktop sends an instance about its application's tools (ADR 0036). Only the desktop
/// sends these, so an instance under the shell never receives one.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    /// A call of one of the application's tools.
    Call(ToolCall),
    /// The agent cancelled the call, or its tool connection closed. An instance gets at most one
    /// cancel per call, and it may still answer.
    Cancel(ToolCallId),
}

/// Identifies a tool call the desktop forwarded to an instance. The desktop chooses it, because the
/// call ids of different tool connections may collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct ToolCallId(u64);

/// A tool call's arguments, a JSON object. They are text only on the wire, and the tool host's
/// connection parses them, within serde_json's nesting limit.
pub type ToolArguments = Map<String, Value>;

/// Decodes a tool call's arguments into the tool's parameter type.
pub fn decode_arguments<Params: DeserializeOwned>(
    arguments: &ToolArguments,
) -> Result<Params, ToolError> {
    Params::deserialize(arguments).map_err(|error| ToolError {
        code: ToolErrorCode::INVALID_ARGUMENT,
        message: format!("The arguments do not match the tool's input schema: {error}."),
    })
}

/// Encodes a tool's result as a JSON value. Integers in a result type fit in 64 bits, because
/// serde_json's values hold no wider ones.
pub fn encode_result(output: &impl Serialize) -> Value {
    serde_json::to_value(output).expect("a tool result type serializes to JSON")
}

/// A failed tool call: an error code and a sentence for the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

/// Why a tool call failed, as a name that starts the error's text for the agent, such as
/// `invalid_argument`.
///
/// Each tool provider defines the codes of its own failures next to its tools and documents them
/// for agents. Agents and their documentation match on the name, so a name never changes once
/// published. The tool protocol reserves the names of its own failures, `connection_closed` and
/// `too_large`, so no provider can use them. Define codes with [`tool_error_code!`], which checks
/// them at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolErrorCode(&'static str);

impl ToolErrorCode {
    /// The arguments break the tool's input schema, or a rule the schema cannot express. The tool
    /// API raises it when it decodes arguments.
    pub const INVALID_ARGUMENT: Self = Self::new("invalid_argument");

    /// Panics unless `code` matches `[a-z][a-z0-9_]*`, which keeps the `code: ` prefix of the
    /// error's text unambiguous, and when the tool protocol reserves `code`. In a const context,
    /// which [`tool_error_code!`] provides, the panic is a compile error at the call. A const panic
    /// message must be a literal, so it names the rule, not the code.
    pub const fn new(code: &'static str) -> Self {
        let bytes = code.as_bytes();
        assert!(
            !bytes.is_empty() && bytes[0].is_ascii_lowercase(),
            "an error code starts with a-z"
        );
        let mut index = 1;
        while index < bytes.len() {
            let byte = bytes[index];
            assert!(
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_',
                "an error code contains only a-z, 0-9 and _"
            );
            index += 1;
        }
        assert!(
            !same_bytes(bytes, CONNECTION_CLOSED.as_bytes())
                && !same_bytes(bytes, TOO_LARGE.as_bytes()),
            "the tool protocol reserves this error code"
        );
        Self(code)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ToolErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A [`ToolErrorCode`], checked at compile time.
///
/// ```
/// const NOT_READY: massive_applications::ToolErrorCode =
///     massive_applications::tool_error_code!("not_ready");
/// ```
///
/// ```compile_fail,E0080
/// let code = massive_applications::tool_error_code!("not-ready");
/// ```
///
/// ```compile_fail,E0080
/// let code = massive_applications::tool_error_code!("too_large");
/// ```
#[macro_export]
macro_rules! tool_error_code {
    ($code:literal) => {
        const { $crate::ToolErrorCode::new($code) }
    };
}

/// Whether `a` and `b` hold the same bytes. Comparing slices with `==` is not possible in a const
/// function.
const fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut index = 0;
    while index < a.len() {
        if a[index] != b[index] {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde_json::{Value, json};

    use super::tool_schema;

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Params {
        count: u32,
        limit: Option<usize>,
        position: Position,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Position {
        column: u32,
    }

    /// The formats go in nested definitions and in optional integers too, which the recursive
    /// transform has to reach.
    #[test]
    fn tool_schemas_carry_no_integer_formats_but_keep_their_minimum() {
        let schema = tool_schema::<Params>().to_value();

        assert_eq!(find_key(&schema, "format"), None);
        assert_eq!(schema["properties"]["count"]["minimum"], json!(0));
        assert_eq!(
            schema["$defs"]["Position"]["properties"]["column"]["minimum"],
            json!(0)
        );
    }

    fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Object(map) => map
                .get(key)
                .or_else(|| map.values().find_map(|value| find_key(value, key))),
            Value::Array(values) => values.iter().find_map(|value| find_key(value, key)),
            _ => None,
        }
    }
}
