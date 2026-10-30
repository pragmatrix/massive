use serde::{Deserialize, Serialize};

/// The first frame of every connection to the desktop socket. Its variant names the protocol the
/// rest of the connection speaks.
///
/// Postcard encodes a variant by its index. A later protocol is therefore added as a new variant
/// at the end. An older desktop cannot decode that variant and closes the connection, which is the
/// right answer for a protocol it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Connect {
    /// A tool client asks to call tools on the MCP root that covers `target`. `version` is the
    /// version of the tool protocol the client speaks for the rest of the connection.
    Tools { version: u16, target: Target },
}

/// The desktop target a tool client connects to. The desktop serves the MCP root that covers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// The id of an instance, as `MASSIVE_INSTANCE` carries it. The client passes the text on
    /// unchanged, and the desktop parses it.
    Instance(String),
    /// A target path, which may name a project or a launcher.
    Path(String),
}
