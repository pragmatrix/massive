//! Calls of applications' tools, which the tool host forwards to instances (ADR 0036).

use std::path::PathBuf;

use massive_applications::{
    AccessScope, ApplicationEvent, CreationMode, McpId, ToolCallId, ToolDeclaration, mcp_id,
    tool_name,
};
use massive_protocol::Frames;
use massive_protocol::tools::v1::{ClientMessage, ConnectReply, HostMessage, ToolOutcome};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use super::tools::configure_mcp;
use super::tools::tool_client::{connect, receive, send};
use super::*;
use crate::Application;
use crate::application_registry::ApplicationRegistry;
use crate::tools::{ToolCatalog, ToolHost};

/// A call of an application's tool reaches the instance its `instance_id` names, without that
/// argument, and the instance's answer returns to the tool client. The connection lists the tool
/// with `instance_id` added to its input schema.
#[tokio::test]
async fn an_application_tool_call_reaches_its_instance_and_returns_its_answer() -> Result<()> {
    task_context::with_context(task_context(), async {
        let mut desktop = EchoDesktop::new()?;
        let socket = desktop.socket.clone();
        let instance = desktop.instance;
        let answer = desktop
            .serve(async {
                let (mut frames, connected) = connect(&socket, "/labs-a").await;
                let ConnectReply::Connected { tools } = connected else {
                    panic!("labs-a is an MCP root");
                };
                let [say] = tools.as_slice() else {
                    panic!("read grants only the echo tool: {tools:?}");
                };
                assert_eq!(say.name, "echo_say");
                let input_schema: serde_json::Value =
                    serde_json::from_str(&say.input_schema).unwrap();
                assert_eq!(
                    input_schema["required"],
                    serde_json::json!(["text", "instance_id"])
                );
                call_echo(&mut frames, 7, instance, "hi").await;
                receive::<HostMessage>(&mut frames).await
            })
            .await;
        assert_eq!(answer, echoed(7, "hi"));
        Ok(())
    })
    .await
}

/// An agent's cancel reaches the instance, and the call stays forwarded, so the answer the instance
/// still gives returns to the agent.
#[tokio::test]
async fn a_cancelled_call_reaches_its_instance_which_may_still_answer() -> Result<()> {
    task_context::with_context(task_context(), async {
        let mut desktop = EchoDesktop::new()?;
        let socket = desktop.socket.clone();
        let instance = desktop.instance;
        let answer = desktop
            .serve(async {
                let (mut frames, _) = connect(&socket, "/labs-a").await;
                call_echo(&mut frames, 7, instance, WAIT).await;
                send(&mut frames, &ClientMessage::CancelTool { call_id: 7 }).await;
                receive::<HostMessage>(&mut frames).await
            })
            .await;
        assert_eq!(answer, echoed(7, CANCELLED));
        Ok(())
    })
    .await
}

/// An instance answers only the calls forwarded to it, so another instance's answer with the same
/// call id never reaches the agent.
#[tokio::test]
async fn an_answer_from_another_instance_does_not_complete_a_call() -> Result<()> {
    task_context::with_context(task_context(), async {
        let mut desktop = EchoDesktop::new()?;
        let socket = desktop.socket.clone();
        let instance = desktop.instance;
        let other_instance = desktop.other_instance;
        let client = async {
            let (mut frames, _) = connect(&socket, "/labs-a").await;
            call_echo(&mut frames, 7, instance, WAIT).await;
            send(&mut frames, &ClientMessage::CancelTool { call_id: 7 }).await;
            receive::<HostMessage>(&mut frames).await
        };
        let answer = desktop
            .serve_with(client, |tool_host| {
                // The first forwarded call gets the id 0.
                let forged = serde_json::json!({ "text": "forged" });
                tool_host.tool_result(other_instance, ToolCallId::from(0), Ok(forged));
            })
            .await;
        assert_eq!(answer, echoed(7, CANCELLED));
        Ok(())
    })
    .await
}

const ECHO: McpId = mcp_id!("echo");
/// The text the echo instance does not answer until the call is cancelled.
const WAIT: &str = "wait";
/// The text the echo instance answers a cancelled call with.
const CANCELLED: &str = "cancelled";

/// A leftover `instance_id` would fail decoding.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EchoParams {
    text: String,
}

#[derive(Serialize, JsonSchema)]
struct Echo {
    text: String,
}

/// The nested siblings with `labs-a` an MCP root that grants `read`, and an echo instance running
/// as `labs-a`'s instance, behind a tool host.
struct EchoDesktop {
    system: DesktopSystem,
    instance: InstanceId,
    /// Another instance the MCP root does not cover, which no task runs.
    other_instance: InstanceId,
    echo_instances: InstanceManager,
    outputs: UnboundedReceiver<(InstanceId, InstanceOutput)>,
    tool_host: ToolHost,
    socket: PathBuf,
    _directory: TempDir,
}

impl EchoDesktop {
    fn new() -> Result<Self> {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        configure_mcp(
            &mut system,
            &mut instance_manager,
            "/labs-a",
            &[AccessScope::Read],
        )?;
        let applications = ApplicationRegistry::new(vec![echo_application()])?;
        let (sender, outputs) = unbounded_channel();
        let mut echo_instances = InstanceManager::new(InstanceEnvironment::new(
            sender,
            1.0,
            Path::new("desktop.sock").into(),
        ));
        // instance_a is presented, but no task runs it, so the echo instance takes its id.
        echo_instances.spawn(
            instance_a,
            applications.get_named("terminal").unwrap(),
            CreationMode::Restore,
            InstanceRoot::new().view_parent(),
        )?;
        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("run").join("desktop.sock");
        let tool_host = ToolHost::listen(socket.clone(), ToolCatalog::new(&applications)?)?;
        Ok(Self {
            system,
            instance: instance_a,
            other_instance: instance_b,
            echo_instances,
            outputs,
            tool_host,
            socket,
            _directory: directory,
        })
    }

    /// Runs the desktop loop's part for tool calls until `client` is done.
    async fn serve<T>(&mut self, client: impl Future<Output = T>) -> T {
        self.serve_with(client, |_| {}).await
    }

    /// Runs the desktop loop's part for tool calls until `client` is done, and calls
    /// `after_request` after each request the tool host answered.
    async fn serve_with<T>(
        &mut self,
        client: impl Future<Output = T>,
        mut after_request: impl FnMut(&mut ToolHost),
    ) -> T {
        tokio::pin!(client);
        loop {
            tokio::select! {
                result = &mut client => return result,
                Some(request) = self.tool_host.next_request() => {
                    let issued = self.tool_host.answer(request, &self.system, &self.echo_instances);
                    assert!(issued.is_none(), "a forwarded call issues no command");
                    after_request(&mut self.tool_host);
                }
                Some((instance, output)) = self.outputs.recv() => {
                    if let InstanceOutput::ToolResult { call_id, outcome } = output {
                        self.tool_host.tool_result(instance, call_id, outcome);
                    }
                }
            }
        }
    }
}

/// The system tests' primary application, which every instance runs. It echoes a call's text, but
/// answers a call with [`WAIT`] only when it is cancelled.
fn echo_application() -> Application {
    Application::new("terminal", |mut ctx| async move {
        let mut waiting = None;
        loop {
            match ctx.wait_for_event().await? {
                ApplicationEvent::ToolCall(call) => {
                    match call.decode().map(|EchoParams { text }| Echo { text }) {
                        Ok(Echo { text }) if text == WAIT => waiting = Some(call.id),
                        echoed => ctx.answer_tool_call(call.id, echoed)?,
                    }
                }
                ApplicationEvent::CancelToolCall(call_id) if waiting == Some(call_id) => {
                    let cancelled = Echo {
                        text: CANCELLED.into(),
                    };
                    ctx.answer_tool_call(call_id, Ok(cancelled))?;
                }
                ApplicationEvent::Shutdown(_) => return Ok(()),
                _ => {}
            }
        }
    })
    .with_mcp_id(
        ECHO,
        vec![ToolDeclaration::new::<EchoParams, Echo>(
            tool_name!(ECHO, "say"),
            AccessScope::Read,
            "Echoes the text.",
        )],
    )
}

async fn call_echo(frames: &mut Frames, call_id: u64, instance: InstanceId, text: &str) {
    let arguments = serde_json::json!({ "instance_id": instance.to_string(), "text": text });
    let call = ClientMessage::CallTool {
        call_id,
        tool: "echo_say".into(),
        arguments: arguments.to_string(),
    };
    send(frames, &call).await;
}

fn echoed(call_id: u64, text: &str) -> HostMessage {
    HostMessage::ToolResult {
        call_id,
        outcome: ToolOutcome::Ok(serde_json::json!({ "text": text }).to_string()),
    }
}
