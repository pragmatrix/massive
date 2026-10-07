//! The desktop configuration document view: the JSON representation, persisted
//! atomically at the end of a transaction.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::persisted::PersistedConfiguration;
use super::write_atomic;
use crate::projects::RuntimeConfiguration;

/// The document view of the desktop configuration: the JSON document, kept in
/// memory and written to disk atomically.
///
/// The document is derived from the aggregate, so a change is applied to the
/// aggregate first and the document is rebuilt from it when persisted — there is no
/// separate edit path to keep in sync. The file is written synchronously, once
/// per transaction, so a transaction's several changes persist as a single
/// write.
#[derive(Debug)]
pub struct ConfigurationPersistence {
    /// Where the configuration lives. Created at load time: the default
    /// configuration is written when no file exists yet, so every later change has
    /// a file to be persisted to.
    path: PathBuf,
    /// Whether applied configuration changes have not yet been written to the file
    /// on disk.
    changed: bool,
}

impl ConfigurationPersistence {
    /// Creates the file view for an existing or soon-to-exist configuration
    /// file. Infallible: reading and parsing happen elsewhere.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            changed: false,
        }
    }
}

impl ConfigurationPersistence {
    /// Marks the file as pending: a configuration change was applied to the
    /// aggregate, and the next persist serializes it into the file.
    ///
    /// The document is derived from the aggregate, so there is no edit step and
    /// nothing to resolve — the call cannot fail, and the change itself is not
    /// needed.
    #[cfg(test)]
    pub fn mark_pending(&mut self) {
        self.changed = true;
    }

    /// Writes the document once if the aggregate has changed since the last
    /// write.
    ///
    /// Called at the end of a transaction, so its several changes persist as one
    /// file write. A write failure is logged and leaves `changed` set, so the next
    /// persist retries it.
    pub fn persist(&mut self, configuration: &RuntimeConfiguration) {
        if !self.changed {
            return;
        }
        let text = match serde_json::to_string_pretty(&PersistedConfiguration::from_configuration(
            configuration,
        )) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("Failed to serialize the configuration: {error}");
                return;
            }
        };
        match write_atomic(&self.path, &text) {
            Ok(()) => {
                self.changed = false;
                log::info!("Configuration persisted to {}", self.path.display());
            }
            Err(error) => log::warn!(
                "Failed to persist configuration to {}: {error:#}",
                self.path.display()
            ),
        }
    }
}

/// Parses the aggregate from configuration text; parse errors fail. Deriving
/// the aggregate cannot: the document is self-contained.
pub fn parse_configuration(path: &Path, text: &str) -> Result<RuntimeConfiguration> {
    let document: PersistedConfiguration =
        serde_json::from_str(text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(document.into_configuration())
}

/// Writes the built-in default configuration to the file, for callers that decide
/// a missing configuration file means "start fresh".
pub fn write_default_config(path: &Path) -> Result<()> {
    let document = PersistedConfiguration::default_document();
    let text =
        serde_json::to_string_pretty(&document).context("serializing the default configuration")?;
    write_atomic(path, &text)
        .with_context(|| format!("writing the default configuration to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::projects::{MatrixPlacement, ProjectId, SlotAssignment};

    /// A change marks the file pending and persisting serializes the aggregate:
    /// the assigned launcher reaches the file.
    #[test]
    fn a_marked_pending_change_persists() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.json");
        let mut configuration = parse_configuration(
            path.as_path(),
            r#"{ "slots": [ { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } } ] }"#,
        )?;
        let mut document = ConfigurationPersistence::new(path.as_path());
        let root = ProjectId::ROOT;

        configuration.assign_slot(
            Some(root),
            MatrixPlacement { column: 1, row: 0 },
            SlotAssignment::Project {
                id: ProjectId::new(),
                name: "extra".into(),
            },
        );
        document.mark_pending();
        document.persist(&configuration);

        let text = fs::read_to_string(&path)?;
        assert!(
            text.contains("\"name\": \"extra\""),
            "unexpected document: {text}"
        );
        Ok(())
    }
}
