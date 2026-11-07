//! The bridge's MCP server, which serves the tools of the bound tool connection to the agent.

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::info;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::{self, IntoTransport};
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
    serve_on(connect(target).await, transport::stdio()).await
}

/// Serves MCP on `transport` for the tool connection `connected`, or offers no tools when the
/// connection failed.
async fn serve_on<T, E, A>(connected: Result<Connected, String>, transport: T) -> Result<()>
where
    T: IntoTransport<RoleServer, E, A>,
    E: Error + Send + Sync + 'static,
{
    let (bridge, ended) = match connected {
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
    let running = bridge.serve(transport).await?;
    match ended {
        Some(ended) => {
            let cancel = running.cancellation_token();
            let mut waiting = pin!(running.waiting());
            tokio::select! {
                result = &mut waiting => { result?; }
                reason = ended => {
                    let reason = reason.context("the tool connection task failed")?;
                    info!("The desktop ended the tool connection: {reason}");
                    // The tool connection failed every pending call. rmcp writes those answers
                    // only while its tasks run, and mt's runtime ends when `serve` returns, so
                    // cancelling and waiting lets rmcp drain them first, as its docs require.
                    cancel.cancel();
                    waiting.await?;
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

/// The desktop socket, from `MASSIVE_SOCKET` or else the desktop's default projects directory.
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

#[cfg(test)]
mod tests {
    use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::runtime;

    use super::*;
    use crate::connection::tests::close_after_one_call;

    /// When the desktop ends the tool connection, the agent still gets the `connection_closed`
    /// answer of its pending call before `serve` returns, as mt's current-thread runtime ends
    /// then. The race this guards against is too narrow to reproduce here, so the test pins the
    /// behavior, not the race.
    #[test]
    fn pending_calls_are_answered_when_the_desktop_ends() {
        let (mut agent, bridge_end) = io::duplex(1 << 16);
        let requests = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"t","arguments":{}}}"#,
        ];
        let runtime = || {
            runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        };

        let bridge_runtime = runtime();
        bridge_runtime.block_on(async {
            for request in requests {
                agent
                    .write_all(format!("{request}\n").as_bytes())
                    .await
                    .unwrap();
            }
            let (connection_end, desktop_end) = UnixStream::pair().unwrap();
            tokio::spawn(close_after_one_call(desktop_end));
            let connected = connection::bind(connection_end, Target::Path("/a".into())).await;
            serve_on(connected, io::split(bridge_end)).await.unwrap();
        });
        // Dropping the runtime drops every task that still holds the transport, as mt's does.
        drop(bridge_runtime);

        let output = runtime().block_on(async {
            let mut output = String::new();
            agent.read_to_string(&mut output).await.unwrap();
            output
        });
        let answer = output
            .lines()
            .find(|line| line.contains(r#""id":2"#))
            .unwrap_or_else(|| panic!("the pending call is answered: {output}"));
        assert!(answer.contains("connection_closed"), "{answer}");
    }
}
