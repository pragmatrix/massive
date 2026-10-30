//! The protocols processes speak with the desktop over the desktop socket (ADR 0036).
//!
//! A connection starts with one [`Connect`] frame, whose variant names the protocol the client
//! speaks. Today the only protocol is the tool protocol, whose messages live in [`tools`]. Every
//! frame is a 4-byte big-endian length followed by the postcard encoding of one message (see
//! [`frames`]).
//!
//! Each version of a protocol is a module that is never edited after its release. A golden test
//! pins the bytes of every message, so an accidental change of the wire format fails CI.

mod connect;
mod frame;
mod socket;
pub mod tools;

pub use connect::{Connect, Target};
pub use frame::{Frames, MAX_FRAME_LENGTH, decode, encode, frames};
pub use socket::desktop_socket_path;
