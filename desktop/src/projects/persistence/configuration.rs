//! The desktop configuration document view: the parsed KDL representation,
//! persisted atomically at the end of a transaction.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use kdl::KdlDocument;

use super::document::{apply_change, atomic_write, configuration_from_document, default_document};
use crate::desktop_system::change::ConfigurationChange;
use crate::projects::DesktopConfiguration;

/// The document view of the desktop configuration: the parsed KDL document, kept
/// in memory and written to disk atomically.
///
/// It holds no configuration of its own: changes arrive together with the
/// aggregate, which supplies the names the document's edits address. The file is
/// written synchronously, once per transaction, so a transaction's several changes
/// persist as a single write.
#[derive(Debug)]
pub struct ConfigurationDocument {
    document: KdlDocument,
    /// Where the configuration lives. Created at load time: the default
    /// configuration is written when no file exists yet, so every later change has
    /// a file to be persisted to.
    path: PathBuf,
    /// Whether applied configuration changes have not yet been written to the file
    /// on disk.
    changed: bool,
}

impl ConfigurationDocument {
    /// Loads the configuration from the file, expecting it to exist, and parses the
    /// aggregate from it.
    ///
    /// Any file error — including a missing file — fails: the caller decides the
    /// no-file policy (see [`write_default_config`]). Configuration parse errors
    /// fail too.
    pub fn load(path: &Path) -> Result<(Self, DesktopConfiguration)> {
        let text = fs::read_to_string(path)?;
        Self::from_str(path, &text)
    }

    /// Parses the configuration from text, deriving the aggregate from it.
    ///
    /// `path` is kept for later persistence. Parse and derivation errors fail.
    pub fn from_str(path: &Path, text: &str) -> Result<(Self, DesktopConfiguration)> {
        let document = text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?;
        let configuration = configuration_from_document(&document)
            .with_context(|| format!("reading configuration from {}", path.display()))?;
        let document = Self {
            document,
            path: path.into(),
            changed: false,
        };
        Ok((document, configuration))
    }
}

/// Writes the built-in default configuration to the file, for callers that decide
/// a missing configuration file means "start fresh".
pub fn write_default_config(path: &Path) -> Result<KdlDocument> {
    let document = default_document();
    atomic_write(path, &document.to_string())
        .with_context(|| format!("writing the default configuration to {}", path.display()))?;
    Ok(document)
}

impl ConfigurationDocument {
    /// Applies a configuration change to the in-memory document, resolving the
    /// names its edits need from `configuration`.
    ///
    /// A rejected change is returned as an error without touching the document, so
    /// callers can report that persistence was rejected and leave the live model
    /// untouched too.
    pub fn apply(
        &mut self,
        change: ConfigurationChange,
        configuration: &DesktopConfiguration,
    ) -> Result<()> {
        self.reject_emptying_removal(&change, configuration)?;
        apply_change(&mut self.document, &change, configuration)?;
        self.changed = true;
        Ok(())
    }

    /// Rejects a removal that would leave the configuration without any launcher:
    /// the next session could not boot into a configuration-defined launcher.
    /// Checked before any edit, so the document stays untouched on rejection.
    fn reject_emptying_removal(
        &self,
        change: &ConfigurationChange,
        configuration: &DesktopConfiguration,
    ) -> Result<()> {
        let removals = match change {
            ConfigurationChange::RemoveLauncher(launcher) => {
                configuration.launcher(*launcher).is_some() as usize
            }
            ConfigurationChange::RemoveProject(project) => configuration
                .project(*project)
                .map(|project| project.launchers().len())
                .unwrap_or(0),
            _ => return Ok(()),
        };
        let remaining = configuration.launcher_count().saturating_sub(removals);
        ensure!(
            remaining > 0,
            "Configuration must define at least one launcher"
        );
        Ok(())
    }

    /// Writes the document once if the in-memory document differs from the file on
    /// disk.
    ///
    /// Called at the end of a transaction, so its several changes persist as one
    /// file write. A write failure is logged and leaves `changed` set, so the next
    /// flush retries it.
    pub fn flush(&mut self) {
        if !self.changed {
            return;
        }
        let text = self.document.to_string();
        match atomic_write(&self.path, &text) {
            Ok(()) => self.changed = false,
            Err(error) => log::warn!(
                "Failed to persist configuration to {}: {error:#}",
                self.path.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document and its derived live model, parsed directly from text — no
    /// temp file needed.
    fn loaded(text: &str) -> Result<(ConfigurationDocument, DesktopConfiguration)> {
        ConfigurationDocument::from_str(Path::new("/config/desktop.kdl"), text)
    }

    /// Regression: removing the last launcher of the last project emptied the
    /// persisted configuration, so the next session failed to boot ("Configuration
    /// must define at least one launcher"). Persistence must reject the change.
    #[test]
    fn removing_last_launcher_is_rejected() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "only" {
    launcher "only" column=0 row=0
}
"#,
        )?;
        let launcher = configuration.projects()[0].launchers()[0].id();

        let error = document
            .apply(
                ConfigurationChange::RemoveLauncher(launcher),
                &configuration,
            )
            .expect_err("removing the last launcher must be rejected");

        assert!(
            error.to_string().contains("at least one launcher"),
            "unexpected error: {error:#}"
        );
        Ok(())
    }

    /// Removing the only project cascades its last launcher away — equally rejected.
    #[test]
    fn removing_last_project_is_rejected() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "only" {
    launcher "only" column=0 row=0
}
"#,
        )?;
        let project = configuration.projects()[0].id();

        let error = document
            .apply(ConfigurationChange::RemoveProject(project), &configuration)
            .expect_err("removing the last project must be rejected");

        assert!(
            error.to_string().contains("at least one launcher"),
            "unexpected error: {error:#}"
        );
        Ok(())
    }
}
