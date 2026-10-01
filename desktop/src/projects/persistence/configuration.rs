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

/// Writes the built-in default configuration to the file, for callers that decide
/// a missing configuration file means "start fresh".
pub fn write_default_config(path: &Path) -> Result<KdlDocument> {
    let document = default_document();
    atomic_write(path, &document.to_string())
        .with_context(|| format!("writing the default configuration to {}", path.display()))?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::projects::{
        LaunchProfile, LaunchProfileId, LauncherMode, MatrixPlacement, Params, ProjectId,
        SlotAssignment,
    };

    /// Two projects sharing a name are two nodes: the change addresses the slot,
    /// so exactly the node the id was tagged on leaves the file.
    #[test]
    fn clearing_a_duplicate_named_project_removes_its_own_node() -> Result<()> {
        let (mut document, _configuration) = loaded(
            r#"
project "work" {
    launcher "first" column=0 row=0
}
project "work" {
    launcher "second" column=0 row=0
}
"#,
        )?;
        let root = ProjectId::ROOT;

        document.apply(ConfigurationChange::ClearSlot {
            parent: root,
            placement: root_row(1),
        })?;

        let remaining = document.document.to_string();
        assert!(
            remaining.contains("\"first\""),
            "unexpected document: {remaining}"
        );
        assert!(
            !remaining.contains("\"second\""),
            "unexpected document: {remaining}"
        );
        Ok(())
    }

    /// The same for launchers: the addressed slot decides which of the two
    /// same-named siblings leaves the file.
    #[test]
    fn clearing_a_duplicate_named_launcher_removes_its_own_node() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "work" {
    launcher "shell" column=0 row=0 command "first"
    launcher "shell" column=1 row=0 command "second"
}
"#,
        )?;
        let work = configuration.projects()[0].id;

        document.apply(ConfigurationChange::ClearSlot {
            parent: work,
            placement: MatrixPlacement { column: 1, row: 0 },
        })?;

        let remaining = document.document.to_string();
        assert!(
            remaining.contains("\"first\""),
            "unexpected document: {remaining}"
        );
        assert!(
            !remaining.contains("\"second\""),
            "unexpected document: {remaining}"
        );
        Ok(())
    }

    /// Applying a change never writes a tag through: the tags live in the node's
    /// source span, so the edited document still holds only the configuration.
    #[test]
    fn tags_do_not_reach_the_file() -> Result<()> {
        let (mut document, _configuration) =
            loaded("project \"work\" {\n    launcher \"shell\" column=0 row=0\n}\n")?;
        let root = ProjectId::ROOT;

        document.apply(ConfigurationChange::AssignSlot {
            parent: root,
            placement: root_row(1),
            assignment: SlotAssignment::Project {
                id: ProjectId::new(),
                name: "extra".into(),
            },
        })?;
        document.apply(ConfigurationChange::ClearSlot {
            parent: root,
            placement: root_row(0),
        })?;

        // The remaining node carries its own placement and none of the tags: a
        // leaked tag would show up as a bare integer argument.
        let text = document.document.to_string();
        assert_eq!(
            text.trim(),
            "project extra column=0 row=1",
            "unexpected document: {text}"
        );
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
        let work = configuration.projects()[0].id;

        document.apply(ConfigurationChange::AssignSlot {
            parent: work,
            placement: MatrixPlacement { column: 2, row: 0 },
            assignment: launcher_assignment("third"),
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
        let (mut document, _configuration) = loaded(
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
        let root = ProjectId::ROOT;

        document.apply(ConfigurationChange::AssignSlot {
            parent: root,
            placement: root_row(2),
            assignment: SlotAssignment::Project {
                id: ProjectId::new(),
                name: "third".into(),
            },
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

    /// A nested project node parses into a nested project with its own slots, and
    /// its launchers are reachable from the aggregate.
    #[test]
    fn nested_project_nodes_parse_into_nested_projects() -> Result<()> {
        let (_document, configuration) = loaded(
            r#"
launcher "top" column=0 row=0

project "labs" column=0 row=1 {
    launcher "shell" column=0 row=0
    project "deep" column=1 row=0 {
        launcher "inner" column=0 row=0
    }
}
"#,
        )?;

        let root = ProjectId::ROOT;
        let labs = configuration
            .child_project_at(root, root_row(1))
            .expect("labs is a root slot");
        let deep = configuration
            .child_project_at(labs, MatrixPlacement { column: 1, row: 0 })
            .expect("deep nests inside labs");

        assert_eq!(configuration.launcher_count(), 3);
        assert_eq!(
            configuration
                .project(deep)
                .map(|project| project.name.as_str()),
            Some("deep")
        );
        Ok(())
    }

    /// A nested project's node keeps its own children when the file is written
    /// back: the tags resolve at any depth.
    #[test]
    fn a_nested_project_keeps_its_subtree_across_an_edit() -> Result<()> {
        let (mut document, configuration) = loaded(
            r#"
project "labs" column=0 row=0 {
    launcher "shell" column=0 row=0
}
"#,
        )?;
        let root = ProjectId::ROOT;
        let labs = configuration
            .child_project_at(root, root_row(0))
            .expect("labs is a root slot");

        document.apply(ConfigurationChange::AssignSlot {
            parent: labs,
            placement: MatrixPlacement { column: 1, row: 0 },
            assignment: launcher_assignment("extra"),
        })?;

        let text = document.document.to_string();
        assert!(text.contains("shell"), "unexpected document: {text}");
        assert!(text.contains("extra"), "unexpected document: {text}");
        Ok(())
    }

    /// An old file without placements migrates to the root slot rows of its former
    /// flat project list, and its launchers keep their own placements.
    #[test]
    fn migration_gives_the_former_project_list_one_row_each() -> Result<()> {
        let (document, configuration) = loaded(
            r#"
project "first" {
    launcher "a" column=0 row=0
}
project "second" {
    launcher "b" column=0 row=0
}
"#,
        )?;

        let root = ProjectId::ROOT;
        assert_eq!(
            configuration.child_project_at(root, root_row(0)),
            Some(configuration.projects()[0].id)
        );
        assert_eq!(
            configuration.child_project_at(root, root_row(1)),
            Some(configuration.projects()[1].id)
        );
        assert!(
            document.document.to_string().contains("row=1"),
            "unexpected document: {}",
            document.document
        );
        Ok(())
    }

    /// Setting the startup launcher keeps the comments above the `startup` node:
    /// the default file's header block rides on that node's leading.
    #[test]
    fn replacing_the_startup_launcher_keeps_the_comments_above_it() -> Result<()> {
        let (mut document, _) = loaded(
            r#"
// the file header

startup "shell"

project "work" {
    launcher "shell" column=0 row=0
}
"#,
        )?;
        document.apply(ConfigurationChange::SetStartupPath(Some(
            "/work/shell".into(),
        )))?;

        let text = document.document.to_string();
        assert!(
            text.contains("// the file header"),
            "unexpected document: {text}"
        );
        assert!(
            text.contains("startup \"/work/shell\""),
            "unexpected document: {text}"
        );
        Ok(())
    }

    /// Setting the startup launcher preserves its address path.
    #[test]
    fn setting_startup_launcher_writes_the_document_path() -> Result<()> {
        let (mut document, _) = loaded(
            r#"
project "work" {
    launcher "shell" column=0 row=0
}
"#,
        )?;
        document.apply(ConfigurationChange::SetStartupPath(Some(
            "/work/shell".into(),
        )))?;

        assert!(
            document
                .document
                .to_string()
                .contains("startup \"/work/shell\""),
            "unexpected document: {}",
            document.document
        );
        Ok(())
    }

    /// A document and its derived live model, parsed directly from text — no
    /// temp file needed.
    fn loaded(text: &str) -> Result<(ConfigurationDocument, DesktopConfiguration)> {
        ConfigurationDocument::from_str(Path::new("/config/desktop.kdl"), text)
    }

    /// The root slot placement a top-level node at `index` migrates to: the former
    /// flat project list keeps its vertical order, one row each.
    fn root_row(index: u32) -> MatrixPlacement {
        MatrixPlacement {
            column: 0,
            row: index,
        }
    }

    fn launcher_assignment(name: &str) -> SlotAssignment {
        SlotAssignment::Launcher {
            id: LaunchProfileId::new(),
            profile: LaunchProfile {
                name: name.into(),
                mode: LauncherMode::Visor,
                params: Params::new(),
            },
        }
    }
}
