use massive_applications::{AccessScope, McpConfiguration};
use massive_protocol::Target;
use massive_protocol::tools::v1::ToolOutcome;

use super::*;
use crate::desktop_system::change::DesktopSystemEffect;

/// Makes the project `path` an MCP root that grants `access_scopes`, and returns the transaction's
/// system effects.
fn configure_mcp(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    path: &str,
    access_scopes: &[AccessScope],
) -> Result<Vec<DesktopSystemEffect>> {
    let project = system
        .aggregates
        .configuration
        .resolve_project_path(ProjectId::ROOT, path)
        .expect("the project resolves");
    let changes = system.plan(DesktopCommand::Configuration(
        ConfigurationCommand::ConfigureMcp {
            project,
            mcp: Some(McpConfiguration {
                access_scopes: access_scopes.iter().copied().collect(),
            }),
        },
    ))?;
    Ok(system
        .transact(changes, instance_manager, TransactionEffectsMode::Setup)?
        .effects)
}

/// A tool connection binds to the MCP root that covers its target. A target outside every MCP root
/// is declined like one that does not exist, and a call needs the access scope of its tool
/// (ADR 0036).
#[tokio::test]
async fn a_tool_connection_reaches_only_the_mcp_root_that_covers_its_target() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        let effects = configure_mcp(&mut system, &mut instance_manager, "/labs-a", &[])?;
        assert!(matches!(
            effects.as_slice(),
            [DesktopSystemEffect::McpRootsChanged(_)]
        ));

        let inside = Target::Instance(instance_a.to_string());
        let outside = Target::Instance(instance_b.to_string());
        let missing = Target::Path("/nowhere".into());
        let bound = system
            .bind_tool_connection(&inside)
            .expect("labs-a covers instance a");
        assert!(bound.tools.is_empty(), "nothing is granted yet");
        assert!(system.bind_tool_connection(&outside).is_none());
        assert!(system.bind_tool_connection(&missing).is_none());
        assert_eq!(
            system
                .bind_tool_connection(&Target::Path("/labs-a/shell-a".into()))
                .map(|bound| bound.root),
            Some(bound.root)
        );

        let refused = system.call_tool(bound.root, "desktop_describe", "{}");
        assert!(matches!(refused, ToolOutcome::Err { code, .. } if code == "scope_not_granted"));

        configure_mcp(
            &mut system,
            &mut instance_manager,
            "/labs-a",
            &[AccessScope::Observe],
        )?;
        let ToolOutcome::Ok(described) = system.call_tool(bound.root, "desktop_describe", "{}")
        else {
            panic!("observe allows describing");
        };
        assert!(described.contains("\"shell-a\""), "{described}");
        assert!(!described.contains("\"shell-b\""), "{described}");

        Ok(())
    })
    .await
}

/// The whole path of a tool connection over a real desktop socket: the connecting, a call answered
/// from the desktop's state, and the closing when the MCP root changes. A target outside every MCP
/// root is declined (ADR 0036).
#[tokio::test]
async fn a_tool_connection_calls_through_the_desktop_socket_and_closes_when_its_root_changes()
-> Result<()> {
    use futures::{SinkExt, StreamExt};
    use massive_protocol::tools::v1::{self, ClientMessage, ConnectReply, HostMessage};
    use massive_protocol::{Connect, Frames, decode, encode, frames};

    use crate::tool_host::ToolHost;

    async fn send(frames: &mut Frames, message: &impl serde::Serialize) {
        frames
            .send(tokio_util::bytes::Bytes::from(encode(message).unwrap()))
            .await
            .unwrap();
    }

    async fn receive<T: serde::de::DeserializeOwned>(frames: &mut Frames) -> T {
        decode(&frames.next().await.unwrap().unwrap()).unwrap()
    }

    async fn connect(socket: &Path, target: &str) -> (Frames, ConnectReply) {
        let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
        let mut frames = frames(stream);
        let connect = Connect::Tools {
            version: v1::VERSION,
            target: Target::Path(target.into()),
        };
        send(&mut frames, &connect).await;
        let reply = receive(&mut frames).await;
        (frames, reply)
    }

    /// Answers the tool host's requests from `system` until `client` is done.
    async fn serve_until<T>(
        tool_host: &mut ToolHost,
        system: &DesktopSystem,
        client: impl Future<Output = T>,
    ) -> T {
        tokio::pin!(client);
        loop {
            tokio::select! {
                result = &mut client => return result,
                Some(request) = tool_host.next_request() => tool_host.answer(request, system),
            }
        }
    }

    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            ..
        } = nested_siblings()?;
        configure_mcp(
            &mut system,
            &mut instance_manager,
            "/labs-a",
            &[AccessScope::Observe],
        )?;
        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("run").join("desktop.sock");
        let mut tool_host = ToolHost::listen(socket.clone())?;

        let (_, declined) = serve_until(&mut tool_host, &system, connect(&socket, "/labs-b")).await;
        assert!(matches!(declined, ConnectReply::Declined { .. }));

        let (mut frames, described) = serve_until(&mut tool_host, &system, async {
            let (mut frames, connected) = connect(&socket, "/labs-a").await;
            let ConnectReply::Connected { tools } = connected else {
                panic!("labs-a is an MCP root");
            };
            assert_eq!(tools.len(), 1);
            let call = ClientMessage::CallTool {
                call_id: 7,
                tool: tools[0].name.clone(),
                arguments: "{}".into(),
            };
            send(&mut frames, &call).await;
            let result = receive(&mut frames).await;
            (frames, result)
        })
        .await;
        let HostMessage::ToolResult {
            call_id: 7,
            outcome: ToolOutcome::Ok(described),
        } = described
        else {
            panic!("the call succeeds: {described:?}");
        };
        assert!(described.contains("\"shell-a\""), "{described}");

        let effects = configure_mcp(&mut system, &mut instance_manager, "/labs-a", &[])?;
        let [DesktopSystemEffect::McpRootsChanged(changed)] = effects.as_slice() else {
            panic!("reconfiguring an MCP root changes it: {effects:?}");
        };
        tool_host.mcp_roots_changed(changed);
        // A connection that is not closed would wait forever, so the wait is bounded.
        let closing = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receive::<HostMessage>(&mut frames),
        )
        .await
        .expect("the desktop closes the connection");
        assert!(matches!(closing, HostMessage::Closing { .. }));

        Ok(())
    })
    .await
}

/// Removing a project that is an MCP root ends the root's tool connections, so the transaction
/// reports the root as changed (ADR 0036).
#[tokio::test]
async fn clearing_a_slot_that_holds_an_mcp_root_reports_the_root_as_changed() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            ..
        } = nested_siblings()?;
        configure_mcp(&mut system, &mut instance_manager, "/labs-a", &[])?;
        let configuration = &system.aggregates.configuration;
        let labs_a = configuration
            .resolve_project_path(ProjectId::ROOT, "/labs-a")
            .expect("labs-a resolves");
        let (parent, slot) = configuration
            .slot_of_content(labs_a)
            .expect("labs-a sits in a slot");

        let changes = system.plan(DesktopCommand::Configuration(
            ConfigurationCommand::ClearSlot {
                parent,
                slot,
                shift: SlotShift::Keep,
            },
        ))?;
        let effects = system
            .transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?
            .effects;

        let [DesktopSystemEffect::McpRootsChanged(changed)] = effects.as_slice() else {
            panic!("removing an MCP root changes it: {effects:?}");
        };
        assert!(changed.contains(&labs_a));
        Ok(())
    })
    .await
}
