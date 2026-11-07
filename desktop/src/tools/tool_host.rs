//! The tool host serves tool connections on the desktop socket (ADR 0036). It is the MCP security
//! boundary: every frame a tool client sends is decoded here, within the frame limit of
//! `massive-protocol`, before anything else runs.
//!
//! The tool host owns no desktop state. A connection task asks the desktop loop to bind it to an
//! MCP root and to carry out its calls, through [`ToolRequest`]s. The tool host forwards the calls
//! of applications' tools to their instances and answers them when the instance does. The desktop
//! loop tells the tool host when MCP roots change, and the tool host closes the connections bound
//! to them.
//!
//! The desktop socket exists while the desktop runs. Only the user can reach it, because its
//! directory has mode `0700`. Without an MCP root, every tool connection is declined.

mod connection;

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use derive_more::From;
use log::{info, warn};
use serde_json::Value;
use tokio::net::UnixListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use massive_applications::{
    InstanceId, InstanceMessage, ToolArguments, ToolCall, ToolCallId, ToolError,
};
use massive_protocol::Target;
use massive_protocol::tools::v1::ToolDescription;
use massive_util::CollectingSet;

use super::{DesktopTools, InstanceCall, ToolCallPlan, error_code};
use crate::desktop_system::{DesktopCommand, DesktopSystem, Refusal};
use crate::instance_manager::InstanceManager;
use crate::projects::ProjectId;
use crate::security::{self, AnsweringCall};

/// A request a tool connection sends to the desktop loop, which answers through `reply`.
#[derive(Debug)]
pub enum ToolRequest {
    Connect {
        target: Target,
        reply: oneshot::Sender<ConnectOutcome>,
    },
    CallTool {
        binding: BindingId,
        call_id: u64,
        tool: String,
        arguments: ToolArguments,
        reply: CallReplySender,
    },
    /// The agent cancelled the call `call_id`.
    CancelTool { binding: BindingId, call_id: u64 },
    /// The connection ended. Its forwarded calls are cancelled.
    Disconnected { binding: BindingId },
}

/// Where the desktop loop sends a call's answer. The connection encodes it for the wire.
type CallReplySender = oneshot::Sender<Result<Value, CallFailure>>;

/// Why a call failed. A tool's own failure carries a [`ToolErrorCode`]. A tool connection that
/// ended first is no failure of any tool, so the connection names it with the tool protocol's
/// `connection_closed` code instead.
#[derive(Debug, From)]
pub enum CallFailure {
    Tool(ToolError),
    #[from(ignore)]
    ConnectionClosed(String),
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
    result: Value,
    reply: CallReplySender,
}

impl CallDone {
    /// Answers the call with its result, once the desktop has carried out its command. A tool
    /// client that left in the meantime no longer listens.
    pub fn answer(self) {
        let _ = self.reply.send(Ok(self.result));
    }

    /// Answers the call with the reason the planner refused its command.
    pub fn refuse(self, refusal: Refusal) {
        let _ = self.reply.send(Err(ToolError {
            code: error_code::REFUSED,
            message: refusal.to_string(),
        }
        .into()));
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

/// What the tool host keeps of a bound connection: its MCP root, and the way to tell it that it
/// ends.
#[derive(Debug)]
struct Binding {
    root: ProjectId,
    closing: oneshot::Sender<String>,
}

/// A call that waits for an instance's answer.
#[derive(Debug)]
struct ForwardedCall {
    binding: BindingId,
    state: ForwardedCallState,
    instance: InstanceId,
    reply: CallReplySender,
}

/// The state of a forwarded call. It waits for the instance's answer in either state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwardedCallState {
    /// The agent may cancel the call by the id its tool client chose, which only its connection
    /// knows.
    Open { client_call_id: u64 },
    /// The agent cancelled the call, and the instance was told. A later cancel, a disconnect, or an
    /// MCP root change does not tell it again.
    Cancelled,
}

/// Serves tool connections and keeps their bindings and forwarded calls. The desktop loop reaches
/// MCP only through it (ADR 0036).
#[derive(Debug)]
pub struct ToolHost {
    socket: Socket,
    /// The requests the connections send to the desktop loop.
    requests: UnboundedReceiver<ToolRequest>,
    bindings: HashMap<BindingId, Binding>,
    next_binding: u64,
    forwarded: HashMap<ToolCallId, ForwardedCall>,
    next_call: u64,
}

/// Whether this desktop serves the desktop socket.
#[derive(Debug)]
enum Socket {
    Served {
        path: PathBuf,
        listener: JoinHandle<()>,
    },
    /// Another desktop serves the socket, so this one offers no tools and leaves the socket alone.
    /// The sender keeps the requests open, so the desktop loop waits for none.
    ServedByAnother {
        _requests: UnboundedSender<ToolRequest>,
    },
}

impl ToolHost {
    /// Creates the desktop socket and listens on it. When another desktop already serves the
    /// socket, this desktop offers no tools, and its agents' tool clients reach the other one.
    pub fn listen(socket_path: PathBuf) -> Result<Self> {
        let run_dir = socket_path
            .parent()
            .context("the desktop socket path has a directory")?;
        fs::create_dir_all(run_dir).with_context(|| format!("creating {}", run_dir.display()))?;
        // Only the user may enter the directory, so other users cannot reach the socket.
        fs::set_permissions(run_dir, fs::Permissions::from_mode(0o700))?;
        let (sender, requests) = unbounded_channel();
        // A desktop that serves the socket accepts the connection. Taking the socket over would cut
        // its agents off, and its drop would delete this desktop's socket.
        if std::os::unix::net::UnixStream::connect(&socket_path).is_ok() {
            warn!(
                "Another desktop serves the desktop socket {}. This desktop offers no MCP tools, \
                 and tool clients reach the other desktop.",
                socket_path.display()
            );
            return Ok(Self::new(
                Socket::ServedByAnother { _requests: sender },
                requests,
            ));
        }
        // A socket file left by a desktop that crashed would make binding fail.
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding {}", socket_path.display()))?;
        info!("Desktop socket listening at {}", socket_path.display());
        let socket = Socket::Served {
            path: socket_path,
            listener: tokio::spawn(accept_connections(listener, sender)),
        };
        Ok(Self::new(socket, requests))
    }

    fn new(socket: Socket, requests: UnboundedReceiver<ToolRequest>) -> Self {
        Self {
            socket,
            requests,
            bindings: HashMap::new(),
            next_binding: 0,
            forwarded: HashMap::new(),
            next_call: 0,
        }
    }

    /// The next request of a tool connection, for the desktop loop to [`Self::answer`]. The
    /// listener holds a sender for as long as the tool host lives, so this never ends.
    pub async fn next_request(&mut self) -> Option<ToolRequest> {
        self.requests.recv().await
    }

    /// Answers a tool connection's request from the desktop's state, forwards a call to the
    /// instance it names, or returns the command a tool call issues, which the desktop loop carries
    /// out before the call is answered. A tool client that left in the meantime no longer listens,
    /// so its answer is dropped.
    pub fn answer(
        &mut self,
        request: ToolRequest,
        system: &DesktopSystem,
        instances: &InstanceManager,
    ) -> Option<IssuedCommand> {
        match request {
            ToolRequest::Connect { target, reply } => {
                let tools = DesktopTools::new(system);
                let outcome = match tools.bind_connection(&target) {
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
                call_id,
                tool,
                arguments,
                reply,
            } => {
                let Some(root) = self.root_of(binding) else {
                    // The MCP root changed after the call was sent, and the connection is ending.
                    let _ = reply.send(Err(CallFailure::ConnectionClosed(
                        "The MCP root's configuration changed.".into(),
                    )));
                    return None;
                };
                let tools = DesktopTools::new(system);
                let catalog = &system.environment().tool_catalog;
                if let Some(application_tool) = catalog.application_tool(&tool) {
                    match tools.forward_call(root, application_tool, arguments) {
                        Ok(call) => self.forward(instances, binding, call_id, call, reply),
                        Err(error) => {
                            let _ = reply.send(Err(error.into()));
                        }
                    }
                    return None;
                }
                match tools.call(root, &tool, &arguments) {
                    ToolCallPlan::Answer(outcome) => {
                        let _ = reply.send(outcome.map_err(CallFailure::from));
                        None
                    }
                    ToolCallPlan::Issue { command, result } => Some(IssuedCommand {
                        command,
                        done: CallDone { result, reply },
                    }),
                }
            }
            ToolRequest::CancelTool { binding, call_id } => {
                // The instance may still answer, so the call stays forwarded until it does.
                let open = ForwardedCallState::Open {
                    client_call_id: call_id,
                };
                let cancelled = self
                    .forwarded
                    .iter_mut()
                    .find(|(_, call)| call.binding == binding && call.state == open);
                if let Some((id, call)) = cancelled {
                    call.state = ForwardedCallState::Cancelled;
                    cancel(instances, call.instance, *id);
                }
                None
            }
            ToolRequest::Disconnected { binding } => {
                self.bindings.remove(&binding);
                self.cancel_forwarded(instances, |call| call.binding == binding);
                None
            }
        }
    }

    /// Answers the forwarded call `call_id` with the outcome `instance` sent. A call that was
    /// cancelled because its connection ended is no longer waited for.
    pub fn tool_result(
        &mut self,
        instance: InstanceId,
        call_id: ToolCallId,
        outcome: Result<Value, ToolError>,
    ) {
        let Entry::Occupied(call) = self.forwarded.entry(call_id) else {
            return;
        };
        match security::answering_call(call.get().instance == instance) {
            AnsweringCall::Accept => {
                let _ = call.remove().reply.send(outcome.map_err(CallFailure::from));
            }
            AnsweringCall::RefuseOtherInstance => {
                warn!("Instance {instance:?} answered a call it did not receive");
            }
        }
    }

    /// Whether the call `call_id` waits for an instance's answer.
    #[cfg(test)]
    pub fn forwards(&self, call_id: ToolCallId) -> bool {
        self.forwarded.contains_key(&call_id)
    }

    /// Fails the calls forwarded to `instance`, which ended before it answered them. The desktop
    /// loop calls this after it received the instance's last output, so no answer is lost.
    pub fn instance_ended(&mut self, instance: InstanceId) {
        for (_, call) in self
            .forwarded
            .extract_if(|_, call| call.instance == instance)
        {
            let _ = call.reply.send(instance_ended());
        }
    }

    /// Ends the connections bound to the `changed` MCP roots, and cancels their forwarded calls.
    pub fn mcp_roots_changed(
        &mut self,
        changed: &CollectingSet<ProjectId>,
        instances: &InstanceManager,
    ) {
        let ended: HashSet<BindingId> = self
            .bindings
            .iter()
            .filter(|(_, binding)| changed.contains(&binding.root))
            .map(|(id, _)| *id)
            .collect();
        for id in &ended {
            let binding = self.bindings.remove(id).expect("an ended binding exists");
            // A connection that ended on its own no longer listens.
            let _ = binding
                .closing
                .send("the MCP root's configuration changed".into());
        }
        self.cancel_forwarded(instances, |call| ended.contains(&call.binding));
    }

    /// Binds a connection to `root`, and returns the binding and its closing receiver.
    fn bind(&mut self, root: ProjectId) -> (BindingId, oneshot::Receiver<String>) {
        let id = BindingId(self.next_binding);
        self.next_binding += 1;
        let (closing, receiver) = oneshot::channel();
        self.bindings.insert(id, Binding { root, closing });
        (id, receiver)
    }

    /// Sends `call` to its instance and waits for the answer. An instance that ended in the
    /// meantime fails the call at once.
    fn forward(
        &mut self,
        instances: &InstanceManager,
        binding: BindingId,
        client_call_id: u64,
        InstanceCall {
            instance,
            tool,
            arguments,
        }: InstanceCall,
        reply: CallReplySender,
    ) {
        let id = ToolCallId::from(self.next_call);
        self.next_call += 1;
        let call = ToolCall::new(id, tool, arguments);
        if instances
            .send_event(instance, InstanceMessage::ToolCall(call))
            .is_err()
        {
            let _ = reply.send(instance_ended());
            return;
        }
        let forwarded = ForwardedCall {
            binding,
            state: ForwardedCallState::Open { client_call_id },
            instance,
            reply,
        };
        self.forwarded.insert(id, forwarded);
    }

    /// Stops waiting for the forwarded calls that `ends` selects, and tells their instances, unless
    /// the agent's cancel already did.
    fn cancel_forwarded(
        &mut self,
        instances: &InstanceManager,
        ends: impl Fn(&ForwardedCall) -> bool,
    ) {
        for (id, call) in self.forwarded.extract_if(|_, call| ends(call)) {
            if let ForwardedCallState::Open { .. } = call.state {
                cancel(instances, call.instance, id);
            }
        }
    }

    /// The MCP root of a binding that still lives.
    fn root_of(&self, binding: BindingId) -> Option<ProjectId> {
        self.bindings.get(&binding).map(|binding| binding.root)
    }
}

impl Drop for ToolHost {
    fn drop(&mut self) {
        let Socket::Served { path, listener } = &self.socket else {
            return;
        };
        listener.abort();
        if let Err(error) = fs::remove_file(path) {
            warn!(
                "Failed to remove the desktop socket {}: {error}",
                path.display()
            );
        }
    }
}

/// Tells `instance` that nobody waits for the call `id` any more. An instance that ended in the
/// meantime no longer listens.
fn cancel(instances: &InstanceManager, instance: InstanceId, id: ToolCallId) {
    let _ = instances.send_event(instance, InstanceMessage::CancelToolCall(id));
}

fn instance_ended() -> Result<Value, CallFailure> {
    Err(ToolError {
        code: error_code::UNKNOWN_ID,
        message: "The instance ended before it answered.".into(),
    }
    .into())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A second desktop leaves the socket to the desktop that serves it, also when it quits, so the
    /// first desktop's agents keep their tools.
    #[tokio::test]
    async fn a_second_desktop_leaves_the_served_socket_alone() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("run").join("desktop.sock");
        let _first = ToolHost::listen(socket.clone()).unwrap();

        let second = ToolHost::listen(socket.clone()).unwrap();
        assert!(matches!(second.socket, Socket::ServedByAnother { .. }));
        drop(second);

        assert!(
            tokio::net::UnixStream::connect(&socket).await.is_ok(),
            "the first desktop still serves the socket"
        );
    }
}
