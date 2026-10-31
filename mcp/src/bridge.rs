//! The bridge's MCP server, which serves the tools of the bound tool connection to the agent.

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::info;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};

use massive_protocol::tools::v1::{ToolDescription, ToolOutcome};
use massive_protocol::{Target, desktop_socket_path};

use crate::connection::{self, Connected, ToolClient};

/// The desktop's projects directory when `MASSIVE_SOCKET` is not set, as the desktop defaults it.
const DEFAULT_PROJECTS_DIR: &str = ".massive";

/// Serves MCP on stdin and stdout until the agent leaves or the desktop ends the tool connection.
/// `target` is a target path from `--target`. Without it, the bridge targets the instance it runs
/// in, from `MASSIVE_INSTANCE`.
pub async fn serve(target: Option<String>) -> Result<()> {
    let (bridge, ended) = match connect(target).await {
        Ok(Connected {
            client,
            tools,
            ended,
        }) => {
            let tools = tools.iter().map(mcp_tool).collect();
            (Bridge::Bound { client, tools }, Some(ended))
        }
        Err(reason) => {
            info!("Offering no tools: {reason}");
            (Bridge::Unbound { reason }, None)
        }
    };
    let running = bridge.serve(rmcp::transport::stdio()).await?;
    match ended {
        Some(ended) => {
            tokio::select! {
                result = running.waiting() => { result?; }
                reason = ended => {
                    let reason = reason.context("the tool connection task failed")?;
                    info!("The desktop ended the tool connection: {reason}");
                }
            }
        }
        None => {
            running.waiting().await?;
        }
    }
    Ok(())
}

/// Connects to the desktop socket and binds the tool connection, or returns why the bridge offers
/// no tools.
async fn connect(target: Option<String>) -> Result<Connected, String> {
    let target = match (target, env::var("MASSIVE_INSTANCE")) {
        (Some(path), _) => Target::Path(path),
        (None, Ok(instance)) => Target::Instance(instance),
        (None, Err(_)) => {
            return Err(
                "this process was not started inside the desktop, and no --target was \
                        given"
                    .into(),
            );
        }
    };
    let socket = socket_path().map_err(|error| format!("{error:#}"))?;
    connection::open(&socket, target).await
}

fn socket_path() -> Result<PathBuf> {
    if let Some(socket) = env::var_os("MASSIVE_SOCKET") {
        return Ok(socket.into());
    }
    let home = env::home_dir().context("no home directory to find the desktop socket in")?;
    Ok(desktop_socket_path(&home.join(DEFAULT_PROJECTS_DIR)))
}

/// The MCP server the agent talks to.
#[derive(Debug)]
enum Bridge {
    /// The tool connection is bound to an MCP root, which grants `tools`.
    Bound {
        client: ToolClient,
        tools: Vec<Tool>,
    },
    /// The bridge offers no tools, because of `reason`, for example because no MCP root covers the
    /// target.
    Unbound { reason: String },
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        let (capabilities, instructions) = match self {
            Self::Bound { .. } => (
                ServerCapabilities::builder().enable_tools().build(),
                "Tools of the MCP root on the massive desktop that this process works in. Ids \
                 are valid only while the desktop runs."
                    .to_string(),
            ),
            // Without the tools capability, MCP clients know that the server offers no tools.
            Self::Unbound { reason } => (
                ServerCapabilities::default(),
                format!("This server offers no tools: {reason}."),
            ),
        };
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("massive", env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = match self {
            Self::Bound { tools, .. } => tools.clone(),
            Self::Unbound { .. } => Vec::new(),
        };
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Self::Bound { client, .. } = self else {
            return Err(ErrorData::invalid_request(
                "this server offers no tools",
                None,
            ));
        };
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let Some(outcome) = client
            .call(request.name.to_string(), arguments.to_string(), context.ct)
            .await
        else {
            // rmcp drops the response to a cancelled request, so the agent never reads this.
            return Err(ErrorData::invalid_request("the call was cancelled", None));
        };
        Ok(mcp_result(outcome).into())
    }
}

/// A tool of the tool list as MCP describes it.
fn mcp_tool(tool: &ToolDescription) -> Tool {
    Tool::new(
        tool.name.clone(),
        tool.description.clone(),
        Arc::new(json_object(&tool.input_schema)),
    )
    .with_raw_output_schema(Arc::new(json_object(&tool.output_schema)))
}

/// The desktop derives every schema from a Rust type, so a schema that is not a JSON object is a
/// bug of the desktop.
fn json_object(schema: &str) -> JsonObject {
    serde_json::from_str(schema).expect("the desktop sends JSON Schemas as JSON objects")
}

/// A tool's result as MCP's structured content, repeated as text. A failure becomes a tool error
/// whose text starts with the error code.
fn mcp_result(outcome: ToolOutcome) -> CallToolResult {
    match outcome {
        ToolOutcome::Ok(result) => match serde_json::from_str(&result) {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "invalid_result: the desktop sent a result that is no JSON: {error}"
            ))]),
        },
        ToolOutcome::Err { code, message } => {
            CallToolResult::error(vec![ContentBlock::text(format!("{code}: {message}"))])
        }
    }
}
