//! The desktop configuration document view: the parsed KDL representation,
//! persisted atomically at the end of a transaction.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use kdl::KdlDocument;

use super::document::{
    NodeTags, apply_change, atomic_write, default_document, parse_configuration,
};
use crate::desktop_system::change::ConfigurationChange;
use crate::projects::DesktopConfiguration;

/// The document view of the desktop configuration: the parsed KDL document, kept
/// in memory and written to disk atomically.
///
/// It is self-contained: changes address the tags it assigns at parse, so the
/// document neither reads the aggregate nor rejects changes on its behalf. The
/// file is written synchronously, once per transaction, so a transaction's several
/// changes persist as a single write.
#[derive(Debug)]
pub struct ConfigurationDocument {
    document: KdlDocument,
    /// The one place a project or launcher id meets the node it names.
    tags: NodeTags,
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
        let mut document: KdlDocument = text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?;
        let (configuration, tags) = parse_configuration(&mut document)
            .with_context(|| format!("reading configuration from {}", path.display()))?;
        let document = Self {
            document,
            tags,
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
    /// Applies a configuration change to the in-memory document.
    ///
    /// An edit that cannot be resolved fails without touching the document, so a
    /// caller can report that persistence was rejected.
    pub fn apply(&mut self, change: ConfigurationChange) -> Result<()> {
        apply_change(&mut self.document, &mut self.tags, &change)?;
        self.changed = true;
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
    use std::path::Path;

    use super::*;
    use crate::projects::{
        LaunchProfile, LaunchProfileId, LauncherMode, MatrixPlacement, Params, ProjectId,
    };

    /// A document and its derived live model, parsed directly from text — no
    /// temp file needed.
    fn loaded(text: &str) -> Result<(ConfigurationDocument, DesktopConfiguration)> {
        ConfigurationDocument::from_str(Path::new("/config/desktop.kdl"), text)
    }

    /// Two projects sharing a name are two nodes: the change's id must remove its
    /// own project, not the first one the name matches.
    #[test]
    fn removing_a_duplicate_named_project_removes_its_own_node() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "work" {
    launcher "first" column=0 row=0
}
project "work" {
    launcher "second" column=0 row=0
}
"#,
        )?;
        let second = configuration.projects()[1].id;

        document.apply(ConfigurationChange::RemoveProject(second))?;

        let remaining = document.document.to_string();
        assert!(remaining.contains("\"first\""));
        assert!(!remaining.contains("\"second\""));
        Ok(())
    }

    /// The same for launchers: the removed launcher's id decides which of the two
    /// same-named siblings leaves the file.
    #[test]
    fn removing_a_duplicate_named_launcher_removes_its_own_node() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "work" {
    launcher "shell" column=0 row=0 command "first"
    launcher "shell" column=1 row=0 command "second"
}
"#,
        )?;
        let second = configuration.projects()[0].launchers()[1].id;

        document.apply(ConfigurationChange::RemoveLauncher(second))?;

        let remaining = document.document.to_string();
        assert!(remaining.contains("\"first\""));
        assert!(!remaining.contains("\"second\""));
        Ok(())
    }

    /// Applying a change never writes a tag through: the tags live in the node's
    /// source span, so the edited document still holds only the configuration.
    #[test]
    fn tags_do_not_reach_the_file() -> Result<()> {
        let (mut document, configuration) =
            loaded("project \"work\" {\n    launcher \"shell\" column=0 row=0\n}\n")?;
        let project = configuration.projects()[0].id;

        document.apply(ConfigurationChange::AddProject {
            id: ProjectId::new(),
            name: "extra".into(),
        })?;
        document.apply(ConfigurationChange::RemoveProject(project))?;

        // A leaked tag shows up as a digit; the remaining project has none.
        let text = document.document.to_string();
        let digits = text.matches(|c: char| c.is_ascii_digit()).count();
        assert_eq!(digits, 0, "unexpected document: {text}");
        assert!(text.contains("extra") && !text.contains("work"));
        Ok(())
    }

    /// An added launcher copies only the whitespace part of its siblings'
    /// spacing: a comment riding on the last child's leading belongs to that
    /// child and must not repeat before the appended node.
    #[test]
    fn adding_a_launcher_does_not_copy_the_comments_before_its_sibling() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "work" {
    launcher "first" column=0 row=0

    // dedicated to first
    launcher "second" column=1 row=0
}
"#,
        )?;
        let project = configuration.projects()[0].id;
        let launcher = LaunchProfileId::new();

        document.apply(ConfigurationChange::AddLauncher {
            project,
            id: launcher,
            profile: LaunchProfile {
                name: "third".into(),
                mode: LauncherMode::Visor,
                params: Params::new(),
            },
            placement: MatrixPlacement { column: 2, row: 0 },
        })?;

        let text = document.document.to_string();
        let comment_count = text.matches("// dedicated to first").count();
        assert_eq!(comment_count, 1, "unexpected document: {text}");
        assert!(
            text.matches("launcher ").count() == 3,
            "unexpected document: {text}"
        );
        assert!(text.contains("third"), "unexpected document: {text}");
        Ok(())
    }

    /// The same for projects: the comment block above the last project stays
    /// that project's own; the appended project gets only the blank line.
    #[test]
    fn adding_a_project_does_not_copy_the_comments_before_its_sibling() -> Result<()> {
        let (mut document, _) = loaded(
            r#"
project "first" {
    launcher "a" column=0 row=0
}

// dedicated to first project
project "second" {
    launcher "b" column=0 row=0
}
"#,
        )?;
        let project_id = ProjectId::new();

        document.apply(ConfigurationChange::AddProject {
            id: project_id,
            name: "third".into(),
        })?;

        let text = document.document.to_string();
        let comment_count = text.matches("// dedicated to first project").count();
        assert_eq!(comment_count, 1, "unexpected document: {text}");
        assert!(
            text.matches("project ").count() == 3,
            "unexpected document: {text}"
        );
        assert!(text.contains("third"), "unexpected document: {text}");
        Ok(())
    }

    /// Setting the startup launcher keeps the comments above the `startup` node:
    /// the default file's header block rides on that node's leading.
    #[test]
    fn replacing_the_startup_launcher_keeps_the_comments_above_it() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
// the file header

startup "shell"

project "work" {
    launcher "shell" column=0 row=0
}
"#,
        )?;
        let launcher = configuration.projects()[0].launchers()[0].id;

        document.apply(ConfigurationChange::SetStartupLauncher(Some(launcher)))?;

        let text = document.document.to_string();
        assert!(
            text.contains("// the file header"),
            "unexpected document: {text}"
        );
        assert!(
            text.contains("startup shell"),
            "unexpected document: {text}"
        );
        Ok(())
    }

    /// Setting the startup launcher writes the node's own name: the change carries
    /// only an id, and the document resolves the name without the aggregate.
    #[test]
    fn setting_startup_launcher_writes_the_document_name() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "work" {
    launcher "shell" column=0 row=0
}
"#,
        )?;
        let launcher = configuration.projects()[0].launchers()[0].id;

        document.apply(ConfigurationChange::SetStartupLauncher(Some(launcher)))?;

        assert!(
            document.document.to_string().contains("startup shell"),
            "unexpected document: {}",
            document.document
        );
        Ok(())
    }
}
