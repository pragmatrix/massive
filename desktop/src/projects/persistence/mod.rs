//! Persistence of the desktop configuration as a hand-editable KDL file.
//!
//! The file is keyed by name: projects and launchers get fresh IDs every startup, so
//! the document cannot reference them. Launcher names must be unique among their
//! project's siblings and project names unique document-wide — a change that would
//! introduce a duplicate is renamed by appending ` 2`, ` 3`, and so on, keeping the
//! live model aligned with the file.

mod configuration;
mod document;
mod parameters;

/// The file name of the desktop configuration inside the projects directory.
pub const CONFIG_FILE_NAME: &str = "desktop.kdl";

pub use self::configuration::{ConfigurationDocument, write_default_config};
