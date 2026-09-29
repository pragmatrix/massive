//! Persistence of the desktop configuration as a hand-editable KDL file.
//!
//! The file is keyed by name — projects and launchers get fresh IDs every startup, so
//! the document cannot reference them — but names may repeat: a node's identity is a
//! tag the document keeps in memory, and every change addresses its node by id.

mod configuration;
mod document;
mod parameters;

/// The file name of the desktop configuration inside the projects directory.
pub const CONFIG_FILE_NAME: &str = "desktop.kdl";

pub use self::configuration::{ConfigurationDocument, write_default_config};
