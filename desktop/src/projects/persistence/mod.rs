//! Persistence of the desktop configuration as a hand-editable JSON file.
//!
//! The file is the fractal view of the aggregate (ADR 0011), derived from it at
//! every persist: a change is applied to the aggregate and the document is
//! serialized from it, so there is no separate edit path to keep in sync.

mod configuration;
mod persisted;

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// The file name of the desktop configuration inside the projects directory.
pub const CONFIG_FILE_NAME: &str = "desktop.json";

pub use self::configuration::{
    ConfigurationPersistence, parse_configuration, write_default_config,
};

/// Writes `text` to `path` atomically: a temp file in the same directory, then a
/// rename over the target.
pub(super) fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
    temp.write_all(text.as_bytes())?;
    temp.persist(path)
        .map_err(|error| anyhow::anyhow!("persisting {}", error.error))?;
    Ok(())
}
