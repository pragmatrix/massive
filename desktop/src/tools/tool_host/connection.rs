//! One tool connection, from its `Connect` frame to its end (ADR 0036).

use anyhow::{Context, Result, bail};
use log::{debug, warn};

use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::UnixStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio_util::bytes::{Bytes, BytesMut};

use massive_applications::{ToolArguments, ToolError, ToolErrorCode};
use massive_protocol::tools::v1::{
    self, ClientMessage, ConnectReply, HostMessage, ToolDescription, ToolOutcome,
};
use massive_protocol::{Connect, Frames, MAX_FRAME_LENGTH, decode, encode, frames};

use super::{BindingId, CallFailure, ConnectOutcome, ToolRequest};

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

    let (reply, outcome) = oneshot::channel();
    requests.send(ToolRequest::Connect { target, reply })?;
    match outcome.await? {
        ConnectOutcome::Bound {
            binding,
            tools,
            closing,
        } => {
            let served = serve_bound(frames, requests, binding, tools, closing).await;
            // A desktop that quits no longer listens.
            let _ = requests.send(ToolRequest::Disconnected { binding });
            served
        }
        ConnectOutcome::Declined { reason } => {
            send(frames, &ConnectReply::Declined { reason }).await
        }
    }
}

async fn serve_bound(
    frames: &mut Frames,
    requests: &UnboundedSender<ToolRequest>,
    binding: BindingId,
    tools: Vec<ToolDescription>,
    closing: oneshot::Receiver<String>,
) -> Result<()> {
    send(frames, &ConnectReply::Connected { tools }).await?;
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
        // A result that is ready goes out before the desktop's closing, so a command that took
        // effect is never reported as closed (see `a_ready_result_goes_out_before_the_closing`).
        tokio::select! {
            biased;
            Some((call_id, outcome)) = pending.next() => {
                frames.send(Bytes::from(result_frame(call_id, received(outcome))?)).await?;
            }
            frame = frames.next() => {
                let Some(frame) = frame else {
                    return Ok(());
                };
                match decode(&frame?)? {
                    ClientMessage::CallTool { call_id, tool, arguments } => {
                        match parse_arguments(&arguments) {
                            Ok(arguments) => {
                                let (reply, outcome) = oneshot::channel();
                                requests.send(ToolRequest::CallTool {
                                    binding,
                                    call_id,
                                    tool,
                                    arguments,
                                    reply,
                                })?;
                                pending.push(async move { (call_id, outcome.await) });
                            }
                            Err(error) => {
                                frames.send(Bytes::from(result_frame(call_id, Err(error.into()))?)).await?;
                            }
                        }
                    }
                    ClientMessage::CancelTool { call_id } => {
                        requests.send(ToolRequest::CancelTool { binding, call_id })?;
                    }
                }
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

/// Parses a call's arguments, which the wire carries as text. serde_json keeps its default nesting
/// limit, so deeply nested arguments fail instead of exhausting the stack.
fn parse_arguments(arguments: &str) -> Result<ToolArguments, ToolError> {
    serde_json::from_str(arguments).map_err(|error| ToolError {
        code: ToolErrorCode::InvalidArgument,
        message: format!("The arguments are not a JSON object: {error}."),
    })
}

/// The outcome of a call, or `connection_closed` when the desktop dropped the call unanswered.
fn received(
    outcome: Result<Result<Value, CallFailure>, oneshot::error::RecvError>,
) -> Result<Value, CallFailure> {
    outcome.unwrap_or_else(|_| {
        Err(CallFailure::ConnectionClosed(
            "The desktop ended the call.".into(),
        ))
    })
}

/// The frame that carries a call's result, encoded as text for the wire. A result too large for
/// one frame fails only its call, because the codec would refuse to write it and end the
/// connection.
fn result_frame(call_id: u64, outcome: Result<Value, CallFailure>) -> Result<Vec<u8>> {
    let outcome = match outcome {
        Ok(result) => ToolOutcome::Ok(result.to_string()),
        Err(CallFailure::Tool(ToolError { code, message })) => ToolOutcome::Err {
            code: code.name().into(),
            message,
        },
        Err(CallFailure::ConnectionClosed(message)) => ToolOutcome::Err {
            code: v1::CONNECTION_CLOSED.into(),
            message,
        },
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool client controls the arguments, so no text may panic or exhaust the stack. Arguments
    /// must be a JSON object, so an array or a number fails too.
    #[test]
    fn arguments_that_are_no_json_object_fail_with_invalid_argument() {
        let deeply_nested = format!(r#"{{"x":{}}}"#, "[".repeat(100_000));
        for arguments in ["", "not json", "[]", "1", &deeply_nested] {
            let error = parse_arguments(arguments).expect_err(arguments);
            assert_eq!(error.code, ToolErrorCode::InvalidArgument);
        }
    }

    /// A result that is ready when the desktop ends the connection still reaches the client, so a
    /// command that took effect is never reported as `connection_closed`. Both are ready at once
    /// here, and the loop repeats, because an unbiased choice would pick either.
    #[tokio::test]
    async fn a_ready_result_goes_out_before_the_closing() {
        for _ in 0..32 {
            let (client_end, host_end) = UnixStream::pair().unwrap();
            let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
            let (closing, closing_receiver) = oneshot::channel();
            tokio::spawn(async move {
                let mut frames = frames(host_end);
                serve_calls(&mut frames, &requests, BindingId(0), closing_receiver).await
            });
            let mut client = frames(client_end);
            let call = ClientMessage::CallTool {
                call_id: 7,
                tool: "t".into(),
                arguments: "{}".into(),
            };
            send(&mut client, &call).await.unwrap();
            let Some(ToolRequest::CallTool { reply, .. }) = received.recv().await else {
                panic!("the call reaches the desktop loop");
            };
            reply.send(Ok(serde_json::json!({}))).unwrap();
            closing.send("closed".into()).unwrap();

            let first: HostMessage = decode(&client.next().await.unwrap().unwrap()).unwrap();
            assert!(
                matches!(first, HostMessage::ToolResult { call_id: 7, .. }),
                "{first:?}"
            );
        }
    }
}
