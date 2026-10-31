//! The tool host serves tool connections on the desktop socket (ADR 0036). It is the MCP security
//! boundary: every frame a tool client sends is decoded here, within the frame limit of
//! `massive-protocol`, before anything else runs.
//!
//! The tool host owns no desktop state. A connection task asks the desktop loop to bind it to an
//! MCP root and to carry out its calls, through [`ToolRequest`]s. The desktop loop tells the tool
//! host when MCP roots change, and the tool host closes the connections bound to them.
//!
//! The desktop socket exists while the desktop runs. Only the user can reach it, because its
//! directory has mode `0700`. Without an MCP root, every tool connection is declined.

mod connection;

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use log::{info, warn};
use tokio::net::UnixListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use massive_applications::{ToolErrorCode, ToolResultJson};
use massive_protocol::Target;
use massive_protocol::tools::v1::{self, ToolDescription, ToolOutcome};
use massive_util::CollectingSet;

use super::{DesktopTools, ToolCallPlan};
use crate::desktop_system::{DesktopCommand, DesktopSystem, Refusal};
use crate::projects::ProjectId;
use crate::security;

/// A request a tool connection sends to the desktop loop, which answers through `reply`.
#[derive(Debug)]
pub enum ToolRequest {
    Connect {
        target: Target,
        reply: oneshot::Sender<ConnectOutcome>,
    },
    CallTool {
        binding: BindingId,
        tool: String,
        arguments: String,
        reply: oneshot::Sender<ToolOutcome>,
    },
}

/// A tool call whose command the desktop loop carries out like a key press, before it answers the
/// call through `done`.
#[derive(Debug)]
pub struct IssuedCommand {
    pub command: DesktopCommand,
    pub done: CallDone,
}

/// The answer of a call whose command the desktop loop carries out.
#[derive(Debug)]
pub struct CallDone {
    result: ToolResultJson,
    reply: oneshot::Sender<ToolOutcome>,
}

impl CallDone {
    /// Answers the call with its result, once the desktop has carried out its command. A tool
    /// client that left in the meantime no longer listens.
    pub fn answer(self) {
        let _ = self.reply.send(ToolOutcome::Ok(self.result.into()));
    }

    /// Answers the call with the reason the planner refused its command.
    pub fn refuse(self, refusal: Refusal) {
        let _ = self.reply.send(ToolOutcome::Err {
            code: ToolErrorCode::Refused.name().into(),
            message: refusal.to_string(),
        });
    }
}

/// What the desktop loop does with a [`ToolRequest::Connect`]. It binds the connection to an MCP
/// root or declines it.
#[derive(Debug)]
pub enum ConnectOutcome {
    /// The connection is bound to an MCP root, whose access scopes grant `tools`. It names its calls
    /// by `binding`, and `closing` fires with a reason when the desktop ends it.
    Bound {
        binding: BindingId,
        tools: Vec<ToolDescription>,
        closing: oneshot::Receiver<String>,
    },
    Declined {
        reason: String,
    },
}

/// Names one binding of a tool connection to an MCP root. A call is carried out only while its
/// binding lives. A call that was already queued when its MCP root changed therefore fails, instead
/// of running with the changed access scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(u64);

#[derive(Debug)]
struct Binding {
    root: ProjectId,
    closing: oneshot::Sender<String>,
}

#[derive(Debug)]
pub struct ToolHost {
    socket_path: PathBuf,
    listener: JoinHandle<()>,
    /// The requests the connections send to the desktop loop.
    requests: UnboundedReceiver<ToolRequest>,
    bindings: HashMap<BindingId, Binding>,
    next_binding: u64,
}

impl ToolHost {
    /// Creates the desktop socket and listens on it.
    pub fn listen(socket_path: PathBuf) -> Result<Self> {
        let run_dir = socket_path
            .parent()
            .context("the desktop socket path has a directory")?;
        fs::create_dir_all(run_dir).with_context(|| format!("creating {}", run_dir.display()))?;
        // Only the user may enter the directory, so other users cannot reach the socket.
        fs::set_permissions(run_dir, fs::Permissions::from_mode(0o700))?;
        // A socket file left by a desktop that crashed would make binding fail.
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding {}", socket_path.display()))?;
        info!("Desktop socket listening at {}", socket_path.display());

        let (sender, requests) = unbounded_channel();
        Ok(Self {
            socket_path,
            listener: tokio::spawn(accept_connections(listener, sender)),
            requests,
            bindings: HashMap::new(),
            next_binding: 0,
        })
    }

    /// The next request of a tool connection, for the desktop loop to [`Self::answer`]. The
    /// listener holds a sender for as long as the tool host lives, so this never ends.
    pub async fn next_request(&mut self) -> Option<ToolRequest> {
        self.requests.recv().await
    }

    /// Answers a tool connection's request from the desktop's state, or returns the command a tool
    /// call issues, which the desktop loop carries out before the call is answered. A tool client
    /// that left in the meantime no longer listens, so its answer is dropped.
    pub fn answer(
        &mut self,
        request: ToolRequest,
        system: &DesktopSystem,
    ) -> Option<IssuedCommand> {
        match request {
            ToolRequest::Connect { target, reply } => {
                let outcome = match DesktopTools::new(system).bind_connection(&target) {
                    Some(bound) => {
                        let (binding, closing) = self.bind(bound.root);
                        ConnectOutcome::Bound {
                            binding,
                            tools: bound.tools,
                            closing,
                        }
                    }
                    None => ConnectOutcome::Declined {
                        reason: security::DECLINED_TARGET.into(),
                    },
                };
                let _ = reply.send(outcome);
                None
            }
            ToolRequest::CallTool {
                binding,
                tool,
                arguments,
                reply,
            } => {
                let plan = match self.root_of(binding) {
                    Some(root) => DesktopTools::new(system).call(root, &tool, &arguments),
                    // The MCP root changed after the call was sent, and the connection is ending.
                    None => ToolCallPlan::Answer(ToolOutcome::Err {
                        code: v1::CONNECTION_CLOSED.into(),
                        message: "The MCP root's configuration changed.".into(),
                    }),
                };
                match plan {
                    ToolCallPlan::Answer(outcome) => {
                        let _ = reply.send(outcome);
                        None
                    }
                    ToolCallPlan::Issue { command, result } => Some(IssuedCommand {
                        command,
                        done: CallDone { result, reply },
                    }),
                }
            }
        }
    }

    /// Ends the connections bound to the `changed` MCP roots.
    pub fn mcp_roots_changed(&mut self, changed: &CollectingSet<ProjectId>) {
        let ended: Vec<BindingId> = self
            .bindings
            .iter()
            .filter(|(_, binding)| changed.contains(&binding.root))
            .map(|(id, _)| *id)
            .collect();
        for id in ended {
            let binding = self.bindings.remove(&id).expect("an ended binding exists");
            // A connection that ended on its own no longer listens.
            let _ = binding
                .closing
                .send("the MCP root's configuration changed".into());
        }
    }

    /// Binds a connection to `root`, and returns the binding and its closing receiver.
    fn bind(&mut self, root: ProjectId) -> (BindingId, oneshot::Receiver<String>) {
        // Connections that ended on their own are forgotten here, so the bindings do not grow.
        self.bindings
            .retain(|_, binding| !binding.closing.is_closed());
        let id = BindingId(self.next_binding);
        self.next_binding += 1;
        let (closing, receiver) = oneshot::channel();
        self.bindings.insert(id, Binding { root, closing });
        (id, receiver)
    }

    /// The MCP root of a binding that still lives.
    fn root_of(&self, binding: BindingId) -> Option<ProjectId> {
        self.bindings.get(&binding).map(|binding| binding.root)
    }
}

impl Drop for ToolHost {
    fn drop(&mut self) {
        self.listener.abort();
        if let Err(error) = fs::remove_file(&self.socket_path) {
            warn!(
                "Failed to remove the desktop socket {}: {error}",
                self.socket_path.display()
            );
        }
    }
}

async fn accept_connections(listener: UnixListener, requests: UnboundedSender<ToolRequest>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(connection::serve(stream, requests.clone()));
            }
            Err(error) => warn!("Failed to accept a tool connection: {error}"),
        }
    }
}
