//! One tool connection, from its `Connect` frame to its end (ADR 0036).

use anyhow::{Context, Result, bail};
use log::{debug, warn};

use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio_util::bytes::{Bytes, BytesMut};

use massive_protocol::tools::v1::{self, ClientMessage, ConnectReply, HostMessage, ToolOutcome};
use massive_protocol::{Connect, Frames, MAX_FRAME_LENGTH, decode, encode, frames};

use super::{BindingId, ConnectAnswer, ToolRequest};

/// Serves one tool connection until either side ends it. Malformed input ends the connection with a
/// log line, never with a panic.
pub async fn serve(stream: UnixStream, requests: UnboundedSender<ToolRequest>) {
    let mut frames = frames(stream);
    if let Err(error) = run(&mut frames, &requests).await {
        debug!("Tool connection ended: {error:#}");
    }
}

async fn run(frames: &mut Frames, requests: &UnboundedSender<ToolRequest>) -> Result<()> {
    let Connect::Tools { version, target } = decode(&next_frame(frames).await?)?;
    if version != v1::VERSION {
        send(
            frames,
            &ConnectReply::Unsupported {
                supported_versions: vec![v1::VERSION],
            },
        )
        .await?;
        bail!("unsupported tool protocol version {version}");
    }

    let (reply, answer) = oneshot::channel();
    requests.send(ToolRequest::Connect { target, reply })?;
    let (binding, closing) = match answer.await? {
        ConnectAnswer::Bound {
            binding,
            tools,
            closing,
        } => {
            let connected = ConnectReply::Connected { tools };
            send(frames, &connected).await?;
            (binding, closing)
        }
        ConnectAnswer::Declined { reason } => {
            send(frames, &ConnectReply::Declined { reason }).await?;
            return Ok(());
        }
    };

    serve_calls(frames, requests, binding, closing).await
}

/// Forwards calls to the desktop loop and their results back, several at a time, until the client
/// leaves or the desktop ends the connection.
async fn serve_calls(
    frames: &mut Frames,
    requests: &UnboundedSender<ToolRequest>,
    binding: BindingId,
    mut closing: oneshot::Receiver<String>,
) -> Result<()> {
    let mut pending = FuturesUnordered::new();
    loop {
        tokio::select! {
            frame = frames.next() => {
                let Some(frame) = frame else {
                    return Ok(());
                };
                match decode(&frame?)? {
                    ClientMessage::CallTool { call_id, tool, arguments } => {
                        let (reply, outcome) = oneshot::channel();
                        requests.send(ToolRequest::CallTool { binding, tool, arguments, reply })?;
                        pending.push(async move { (call_id, outcome.await) });
                    }
                    // Desktop tools finish within one desktop loop turn, so there is nothing to
                    // cancel yet. Application tools will forward it (ADR 0036).
                    ClientMessage::CancelTool { .. } => {}
                }
            }
            Some((call_id, outcome)) = pending.next() => {
                let outcome = outcome.unwrap_or_else(|_| ToolOutcome::Err {
                    code: v1::CONNECTION_CLOSED.into(),
                    message: "The desktop ended the call.".into(),
                });
                frames.send(Bytes::from(result_frame(call_id, outcome)?)).await?;
            }
            reason = &mut closing => {
                let reason = reason.unwrap_or_else(|_| "the desktop quits".into());
                if let Err(error) = send(frames, &HostMessage::Closing { reason }).await {
                    warn!("Failed to tell a tool client why its connection ends: {error:#}");
                }
                return Ok(());
            }
        }
    }
}

/// The frame that carries a call's result. A result too large for one frame fails only its call,
/// because the codec would refuse to write it and end the connection.
fn result_frame(call_id: u64, outcome: ToolOutcome) -> Result<Vec<u8>> {
    let frame = encode(&HostMessage::ToolResult { call_id, outcome })?;
    if frame.len() <= MAX_FRAME_LENGTH {
        return Ok(frame);
    }
    let outcome = ToolOutcome::Err {
        code: v1::TOO_LARGE.into(),
        message: format!("The result exceeds the limit of {MAX_FRAME_LENGTH} bytes."),
    };
    Ok(encode(&HostMessage::ToolResult { call_id, outcome })?)
}

async fn next_frame(frames: &mut Frames) -> Result<BytesMut> {
    frames
        .next()
        .await
        .context("the tool client left before connecting")?
        .context("reading a frame")
}

async fn send(frames: &mut Frames, message: &impl serde::Serialize) -> Result<()> {
    frames.send(Bytes::from(encode(message)?)).await?;
    Ok(())
}
