use serde::{Deserialize, Serialize};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

/// The size limit of a frame's payload, in both directions. It is the protocol's only size limit:
/// a tool's arguments or result cannot exceed the frame that carries them.
pub const MAX_FRAME_LENGTH: usize = 1024 * 1024;

/// A connection on the desktop socket, read as a stream of frame payloads and written as a sink of
/// them. Both the desktop and its clients use it, so they split the byte stream the same way.
pub type Frames = Framed<UnixStream, LengthDelimitedCodec>;

/// Reads and writes `stream` as frames.
pub fn frames(stream: UnixStream) -> Frames {
    Framed::new(stream, frame_codec())
}

/// The codec that splits the connection's byte stream into frames. It rejects a frame whose length
/// prefix exceeds [`MAX_FRAME_LENGTH`] before reading its payload, so a peer cannot make the other
/// side allocate more than that. It also refuses to write a larger frame.
fn frame_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .big_endian()
        .max_frame_length(MAX_FRAME_LENGTH)
        .new_codec()
}

/// The payload of one frame.
pub fn encode(message: &impl Serialize) -> postcard::Result<Vec<u8>> {
    postcard::to_stdvec(message)
}

/// Decodes the payload of one frame. Malformed or trailing bytes are an error, never a panic.
pub fn decode<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> postcard::Result<T> {
    let (message, rest) = postcard::take_from_bytes(payload)?;
    if !rest.is_empty() {
        return Err(postcard::Error::DeserializeBadEncoding);
    }
    Ok(message)
}
