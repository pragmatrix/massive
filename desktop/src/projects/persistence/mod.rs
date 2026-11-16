//! Persistence of the desktop configuration as a hand-editable KDL file (ADR 0037).
//!
//! The file is the fractal view of the aggregate (ADR 0011), derived from it at every persist. A
//! change is applied to the aggregate, and the document derived from it is reconciled into the
//! file on disk, so there is no separate edit path to keep in sync and the file keeps its comments
//! and formatting.

mod configuration;
mod kdl_format;
mod persisted;
mod reconcile;
mod write_atomic;

/// The file name of the desktop configuration inside the projects directory.
pub const CONFIG_FILE_NAME: &str = "desktop.kdl";

pub use self::configuration::{
    ConfigurationPersistence, parse_configuration, write_default_config,
};
use self::write_atomic::write_atomic;
