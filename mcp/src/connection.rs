//! The bridge's tool connection to the desktop (ADR 0036).
//!
//! One task owns the connection and all of its state. It sends the calls, routes each result to
//! its call, and answers every call it still holds when the connection ends. A second task only
//! writes the frames the first one queues, so the first keeps reading while a large frame is
//! written. The bridge reaches the connection only through a [`ToolClient`], so no state is shared
//! and nothing is locked.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;

use futures::stream::{FuturesUnordered, SplitSink};
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::bytes::Bytes;
use tokio_util::sync::CancellationToken;

use massive_protocol::tools::v1::{
    self, ClientMessage, ConnectReply, HostMessage, ToolDescription, ToolOutcome,
};
use massive_protocol::{Connect, Frames, MAX_FRAME_LENGTH, Target, decode, encode, frames};

/// A tool connection bound to an MCP root, the tools the root grants, and the task that serves it.
#[derive(Debug)]
pub struct Connected {
    pub client: ToolClient,
    pub tools: Vec<ToolDescription>,
    /// Resolves with the reason once the connection has ended.
    pub ended: JoinHandle<String>,
}

/// The bridge's handle on a bound tool connection. It holds none of the connection's state, so
/// calls run at the same time without locking.
#[derive(Debug, Clone)]
pub struct ToolClient {
    calls: mpsc::UnboundedSender<Call>,
}

/// A call the client hands to the connection task, which answers it through `reply`.
#[derive(Debug)]
struct Call {
    tool: String,
    arguments: String,
    cancelled: CancellationToken,
    reply: oneshot::Sender<ToolOutcome>,
}

/// Connects to the desktop socket and asks to be bound to the MCP root that covers `target`.
/// The error is the reason the bridge offers no tools.
pub async fn open(socket: &Path, target: Target) -> Result<Connected, String> {
    let stream = UnixStream::connect(socket).await.map_err(connect_failed)?;
    bind(stream, target).await
}

impl ToolClient {
    /// Calls a tool and waits for its outcome, or returns `None` when the agent cancelled the call.
    /// The connection task tells the desktop about a cancellation.
    pub async fn call(
        &self,
        tool: String,
        arguments: String,
        cancelled: CancellationToken,
    ) -> Option<ToolOutcome> {
        let (reply, outcome) = oneshot::channel();
        let call = Call {
            tool,
            arguments,
            cancelled,
            reply,
        };
        if self.calls.send(call).is_err() {
            return Some(connection_closed("the desktop connection ended"));
        }
        // The connection task answers every call but a cancelled one, whose reply it drops.
        outcome.await.ok()
    }
}

/// Why the bridge cannot reach the desktop. A missing socket and a refused connection both mean
/// that no desktop is running.
fn connect_failed(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
            "the desktop is not running".into()
        }
        _ => format!("the desktop socket cannot be reached: {error}"),
    }
}

/// Sends the `Connect` frame for `target` over `stream`, and starts the connection task once the
/// desktop has bound the connection.
async fn bind(stream: UnixStream, target: Target) -> Result<Connected, String> {
    let mut frames = frames(stream);
    let connect = Connect::Tools {
        version: v1::VERSION,
        target,
    };
    send(&mut frames, &connect)
        .await
        .map_err(handshake_failed)?;
    let reply = frames
        .next()
        .await
        .ok_or_else(|| handshake_failed("the desktop closed the connection"))?
        .map_err(handshake_failed)?;
    let tools = match decode(&reply).map_err(handshake_failed)? {
        ConnectReply::Connected { tools } => tools,
        ConnectReply::Declined { reason } => return Err(reason),
        ConnectReply::Unsupported { supported_versions } => {
            return Err(format!(
                "the desktop speaks the tool protocol versions {supported_versions:?}, but this \
                 bridge speaks version {}",
                v1::VERSION
            ));
        }
    };

    let (calls, receiver) = mpsc::unbounded_channel();
    Ok(Connected {
        client: ToolClient { calls },
        tools,
        ended: tokio::spawn(serve(frames, receiver)),
    })
}

/// Why the bridge offers no tools when the exchange of `Connect` and its reply fails.
fn handshake_failed(error: impl fmt::Display) -> String {
    format!("the desktop connection failed: {error}")
}

/// The connection task. It serves the calls until the connection ends, then answers every call it
/// still holds with `connection_closed` and returns the reason.
async fn serve(frames: Frames, mut calls: mpsc::UnboundedReceiver<Call>) -> String {
    // Writing runs in its own task, so reading never waits for a write. Otherwise a large call
    // and a large result, written at the same time, would deadlock (see the test
    // `a_large_call_and_a_large_result_cross_without_blocking_each_other`).
    let (sink, mut incoming) = frames.split();
    let (outgoing, queued) = mpsc::unbounded_channel();
    let mut writer = tokio::spawn(write_frames(sink, queued));
    // The route of each call the desktop has not answered yet, by call id.
    let mut routes: HashMap<u64, oneshot::Sender<ToolOutcome>> = HashMap::new();
    let mut in_flight = FuturesUnordered::new();
    let mut next_call_id = 0;

    let reason = loop {
        tokio::select! {
            Some(call) = calls.recv() => {
                let call_id = next_call_id;
                next_call_id += 1;
                let frame = frame(&ClientMessage::CallTool {
                    call_id,
                    tool: call.tool,
                    arguments: call.arguments,
                });
                if frame.len() > MAX_FRAME_LENGTH {
                    // The caller may have left in the meantime.
                    let _ = call.reply.send(too_large());
                    continue;
                }
                let (route, routed) = oneshot::channel();
                routes.insert(call_id, route);
                in_flight.push(forward_outcome(call_id, routed, call.cancelled, call.reply));
                // A writer that failed is reported by its own branch.
                let _ = outgoing.send(frame);
            }
            frame = incoming.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(error)) => break format!("reading from the desktop failed: {error}"),
                    None => break "the desktop closed the connection".to_string(),
                };
                match decode(&frame) {
                    Ok(HostMessage::ToolResult { call_id, outcome }) => {
                        // A cancelled call has no route anymore, and one that was cancelled just
                        // now no longer receives.
                        if let Some(route) = routes.remove(&call_id) {
                            let _ = route.send(outcome);
                        }
                    }
                    Ok(HostMessage::Closing { reason }) => break reason,
                    Err(error) => break format!("the desktop sent a malformed message: {error}"),
                }
            }
            Some(cancelled) = in_flight.next() => {
                let Some(call_id) = cancelled else {
                    continue;
                };
                routes.remove(&call_id);
                // A writer that failed is reported by its own branch.
                let _ = outgoing.send(frame(&ClientMessage::CancelTool { call_id }));
            }
            written = &mut writer => {
                break written.expect("the writer task does not panic");
            }
        }
    };

    // Every call still gets an answer. First, each call the desktop has not answered is routed
    // `connection_closed`.
    for (_, route) in routes.drain() {
        let _ = route.send(connection_closed(&reason));
    }
    // Then the routed outcomes reach their callers, because `forward_outcome` runs only while this
    // task polls it.
    while in_flight.next().await.is_some() {}
    // Last, the calls still queued are answered. After the close, `ToolClient::call` answers new
    // calls itself.
    calls.close();
    while let Some(call) = calls.recv().await {
        let _ = call.reply.send(connection_closed(&reason));
    }
    reason
}

/// The writer task. It writes the queued frames in order, and returns why it stopped: a failed
/// write, or the end of the connection task, which drops the queue.
async fn write_frames(
    mut sink: SplitSink<Frames, Bytes>,
    mut queued: mpsc::UnboundedReceiver<Bytes>,
) -> String {
    while let Some(frame) = queued.recv().await {
        if let Err(error) = sink.send(frame).await {
            return format!("writing to the desktop failed: {error}");
        }
    }
    "the connection ended".into()
}

/// Encodes one message as the payload of one frame.
fn frame(message: &impl serde::Serialize) -> Bytes {
    Bytes::from(encode(message).expect("a tool protocol message encodes"))
}

/// Forwards a call's outcome to its caller, or returns the call's id when the agent cancels the call
/// first. The connection task polls it, so the caller hears nothing until the task does.
async fn forward_outcome(
    call_id: u64,
    routed: oneshot::Receiver<ToolOutcome>,
    cancelled: CancellationToken,
    reply: oneshot::Sender<ToolOutcome>,
) -> Option<u64> {
    tokio::select! {
        outcome = routed => {
            let outcome = outcome.expect("the connection task answers every route before dropping it");
            // The caller may have left in the meantime.
            let _ = reply.send(outcome);
            None
        }
        _ = cancelled.cancelled() => Some(call_id),
    }
}

/// Writes one message as one frame. The error is the reason the connection ends.
async fn send(frames: &mut Frames, message: &impl serde::Serialize) -> Result<(), String> {
    let frame = encode(message).map_err(|error| error.to_string())?;
    frames
        .send(Bytes::from(frame))
        .await
        .map_err(|error| error.to_string())
}

/// The outcome of a call that the connection can no longer answer (ADR 0036).
fn connection_closed(reason: &str) -> ToolOutcome {
    ToolOutcome::Err {
        code: v1::CONNECTION_CLOSED.into(),
        message: format!("{reason}."),
    }
}

/// The outcome of a call whose arguments the desktop would refuse to read (see
/// [`MAX_FRAME_LENGTH`]).
fn too_large() -> ToolOutcome {
    ToolOutcome::Err {
        code: v1::TOO_LARGE.into(),
        message: format!("The arguments exceed the limit of {MAX_FRAME_LENGTH} bytes."),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn closing_answers_the_call_in_flight_and_every_later_call() {
        let (bridge_end, desktop_end) = UnixStream::pair().unwrap();
        tokio::spawn(close_after_one_call(desktop_end));
        let connected = bind(bridge_end, Target::Path("/a".into())).await.unwrap();
        let call = || {
            connected
                .client
                .call("t".into(), "{}".into(), CancellationToken::new())
        };

        assert_eq!(call().await, Some(connection_closed("closed")));
        assert_eq!(connected.ended.await.unwrap(), "closed");
        assert_eq!(
            call().await,
            Some(connection_closed("the desktop connection ended"))
        );
    }

    /// A large call and a large result cross. The desktop's tool host does not read while it writes
    /// a result, so the bridge must keep reading while it writes a call, or neither write ends,
    /// because each fills the socket buffers the other side no longer empties.
    #[tokio::test]
    async fn a_large_call_and_a_large_result_cross_without_blocking_each_other() {
        let (bridge_end, desktop_end) = UnixStream::pair().unwrap();
        tokio::spawn(answer_with_a_large_result(desktop_end));
        let connected = bind(bridge_end, Target::Path("/a".into())).await.unwrap();
        let call = |arguments: String| {
            connected
                .client
                .call("t".into(), arguments, CancellationToken::new())
        };

        // `join!` polls the small call first, so it is sent first and gets the large result.
        let both = async { tokio::join!(call("{}".into()), call(large_text())) };
        let (small, large) = tokio::time::timeout(Duration::from_secs(5), both)
            .await
            .expect("the large call and the large result block each other");
        assert_eq!(small, Some(ToolOutcome::Ok(large_text())));
        assert_eq!(large, Some(ToolOutcome::Ok("{}".into())));
    }

    /// The desktop's side of one connection: it binds the connection, receives one call, and ends
    /// the connection without answering it.
    async fn close_after_one_call(stream: UnixStream) {
        let mut frames = frames(stream);
        let _: Connect = decode(&frames.next().await.unwrap().unwrap()).unwrap();
        send(&mut frames, &ConnectReply::Connected { tools: Vec::new() })
            .await
            .unwrap();
        let _: ClientMessage = decode(&frames.next().await.unwrap().unwrap()).unwrap();
        send(
            &mut frames,
            &HostMessage::Closing {
                reason: "closed".into(),
            },
        )
        .await
        .unwrap();
    }

    /// The desktop's side of one connection, written like the tool host's loop: it answers the
    /// first call with a large result, writing it without reading, then answers every further call
    /// with `{}`.
    async fn answer_with_a_large_result(stream: UnixStream) {
        let mut frames = frames(stream);
        let _: Connect = decode(&frames.next().await.unwrap().unwrap()).unwrap();
        send(&mut frames, &ConnectReply::Connected { tools: Vec::new() })
            .await
            .unwrap();
        let mut outcome = ToolOutcome::Ok(large_text());
        while let Some(frame) = frames.next().await {
            let ClientMessage::CallTool { call_id, .. } = decode(&frame.unwrap()).unwrap() else {
                continue;
            };
            let outcome = std::mem::replace(&mut outcome, ToolOutcome::Ok("{}".into()));
            send(&mut frames, &HostMessage::ToolResult { call_id, outcome })
                .await
                .unwrap();
        }
    }

    /// Far more than the socket buffers hold, which are 8 KiB in each direction on macOS, and
    /// well within the frame limit.
    fn large_text() -> String {
        "x".repeat(512 * 1024)
    }
}
