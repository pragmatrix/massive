//! The desktop configuration file view: loading the aggregate from the KDL file and persisting it
//! there atomically at the end of a transaction.

use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{Context, Result, ensure};
use kdl::KdlDocument;

use super::persisted::PersistedConfiguration;
use super::reconcile::reconcile;
use super::{kdl_format, write_atomic};
use crate::policy::constants;
use crate::policy::when::{self, NestingProjects, PersistingOverUnloadableFile};
use crate::projects::RuntimeConfiguration;

/// The file view of the desktop configuration: the path the KDL document, derived from the
/// aggregate, is written to atomically.
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
}

impl ConfigurationPersistence {
    /// Creates the file view for an existing or soon-to-exist configuration
    /// file. Infallible: reading and parsing happen elsewhere.
    pub fn new(path: &Path) -> Self {
        Self { path: path.into() }
    }

    /// Writes the document derived from the aggregate, reconciled into the file on disk, so the
    /// file keeps its comments and formatting (ADR 0037).
    ///
    /// Called only for a transaction that changed the configuration, so its several changes
    /// persist as one file write. The file is read again each time, so comments added while the
    /// terminal runs survive. A file that does not load is left alone, so a half-finished hand
    /// edit survives too. Each failure is logged as an error, and the next change writes the whole
    /// aggregate again.
    pub fn persist(&self, configuration: &RuntimeConfiguration) {
        let tree = PersistedConfiguration::from_configuration(configuration);
        let fresh = kdl_format::write_document(&tree);
        let document = match self.read_existing_document() {
            Ok(Some(existing)) => reconcile(existing, fresh),
            Ok(None) => fresh,
            Err(error) => match when::persisting_over_unloadable_file() {
                PersistingOverUnloadableFile::KeepFile => {
                    log::error!(
                        "Configuration changes are not saved until the file loads again: {error:#}"
                    );
                    return;
                }
            },
        };
        let text = document.to_string();
        if let Err(error) = self.check_reads_back(&text, &tree) {
            log::error!(
                "Configuration changes are not saved, the document to write does not read back: \
                 {error:#}"
            );
            return;
        }
        match write_atomic(&self.path, &text) {
            Ok(()) => log::info!("Configuration persisted to {}", self.path.display()),
            Err(error) => log::error!(
                "Failed to persist configuration to {}: {error:#}",
                self.path.display()
            ),
        }
    }

    /// Parses the text about to be written, in memory, and checks that it reads as `tree`. A
    /// mismatch is a bug in the reconcile or in `kdl`, and writing the text would carry it into the
    /// user's file, where the next start would load a different configuration or none.
    fn check_reads_back(&self, text: &str, tree: &PersistedConfiguration) -> Result<()> {
        let read = kdl_format::read_configuration(&self.path, text)?;
        ensure!(
            read == *tree,
            "the reconciled document reads as a different configuration"
        );
        Ok(())
    }

    /// Reads and parses the file on disk, and checks it as the load does. An unknown node passes
    /// the KDL parser, but the reconcile would remove it. `None` when the file does not exist.
    fn read_existing_document(&self) -> Result<Option<KdlDocument>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", self.path.display()));
            }
        };
        let document = kdl_format::parse_document(&self.path, &text)?;
        read_checked(&self.path, &text, &document)?;
        Ok(Some(document))
    }
}

/// Parses the aggregate from configuration text; parse errors fail. Deriving
/// the aggregate cannot: the document is self-contained.
///
/// A configuration that nests projects deeper than the desktop allows fails to load, with an error
/// that names the depth and the limit. Every change keeps within the limits, so only a hand edit gets
/// there, and the user fixes the file.
pub fn parse_configuration(path: &Path, text: &str) -> Result<RuntimeConfiguration> {
    let document = kdl_format::parse_document(path, text)?;
    Ok(read_checked(path, text, &document)?.into_configuration())
}

/// Reads the typed tree from a document parsed from `text` and checks the nesting limit. The load
/// and the persist share it, so a persist leaves alone exactly the files the load refuses.
fn read_checked(path: &Path, text: &str, document: &KdlDocument) -> Result<PersistedConfiguration> {
    let tree = kdl_format::read_document(path, text, document)?;
    let deepest_project = tree.project_depth();
    ensure!(
        when::nesting_projects(Some(deepest_project)) == NestingProjects::Allow,
        "{} nests projects {deepest_project} levels deep, at most {} are allowed",
        path.display(),
        constants::configuration::MAX_PROJECT_DEPTH
    );
    Ok(tree)
}

/// Writes the built-in default configuration to the file, for callers that decide
/// a missing configuration file means "start fresh".
pub fn write_default_config(path: &Path) -> Result<()> {
    write_atomic(path, DEFAULT_CONFIG)
        .with_context(|| format!("writing the default configuration to {}", path.display()))
}

/// The built-in default configuration. It is written as text, so its comments introduce the format
/// to a user who opens the file.
const DEFAULT_CONFIG: &str = r#"// The desktop configuration: projects, launchers, and the launcher the session boots into.
// The terminal writes its changes to this file and keeps comments and formatting.
//
// `at` is a slot's position as "column,row". The child nodes of a launcher are its spawn
// parameters, for example `command "ssh home"`.

startup "/Primary / Local/Primary / Local"

project "Primary / Local" at="0,0" {
    launcher "Primary / Local" at="0,0"
}
project default at="0,1" {
    launcher default at="0,0"
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{ProjectId, SlotAssignment, SlotPosition};

    /// A configuration whose projects nest as deep as the limit allows saves and loads again, so
    /// no configuration the desktop accepts keeps it from starting. A change to the saved format
    /// that adds nesting levels fails here first.
    #[test]
    fn a_configuration_at_the_nesting_limit_saves_and_loads() -> Result<()> {
        use crate::policy::constants::configuration::MAX_PROJECT_DEPTH;
        use crate::projects::{LaunchProfile, LaunchProfileId};

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.kdl");
        let mut configuration = parse_configuration(path.as_path(), "")?;
        let first = SlotPosition { column: 0, row: 0 };
        let mut deepest = ProjectId::ROOT;
        for _ in 0..MAX_PROJECT_DEPTH {
            let nested = ProjectId::new();
            configuration.assign_slot(
                Some(deepest),
                first,
                SlotAssignment::Project {
                    id: nested,
                    name: "nested".into(),
                },
            );
            deepest = nested;
        }
        let params = serde_json::Map::from_iter([("command".to_string(), "just".into())]);
        configuration.assign_slot(
            Some(deepest),
            first,
            SlotAssignment::Launcher {
                id: LaunchProfileId::new(),
                profile: LaunchProfile {
                    name: "shell".into(),
                    params,
                    full_screen_mode: Default::default(),
                },
            },
        );
        assert_eq!(configuration.project_depth(deepest), MAX_PROJECT_DEPTH);

        ConfigurationPersistence::new(path.as_path()).persist(&configuration);
        parse_configuration(path.as_path(), &fs::read_to_string(&path)?)?;
        Ok(())
    }

    /// A file that nests deeper than the limit fails to load, and the error names the depth, so
    /// the user can find and fix it. Only a hand edit gets a file there.
    #[test]
    fn a_configuration_past_the_nesting_limit_fails_to_load() {
        use crate::policy::constants::configuration::MAX_PROJECT_DEPTH;

        let path = Path::new("desktop.kdl");
        let nested = |depth: usize, innermost: String| {
            (0..depth).fold(innermost, |inner, _| {
                format!("project p at=\"0,0\" {{ {inner} }}")
            })
        };

        let too_deep = nested(MAX_PROJECT_DEPTH + 1, "launcher shell at=\"0,0\"".into());
        let error = parse_configuration(path, &too_deep).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("{} levels deep", MAX_PROJECT_DEPTH + 1)),
            "{error}"
        );
    }

    /// The built-in default configuration is text, so only a test catches a mistake in it before a
    /// fresh installation fails to start.
    #[test]
    fn the_default_configuration_loads() -> Result<()> {
        parse_configuration(Path::new("desktop.kdl"), DEFAULT_CONFIG)?;
        Ok(())
    }

    /// Persisting writes the aggregate as it is: the assigned project reaches the file.
    #[test]
    fn a_persisted_change_reaches_the_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.kdl");
        let mut configuration = parse_configuration(path.as_path(), r#"launcher shell at="0,0""#)?;
        let document = ConfigurationPersistence::new(path.as_path());
        let root = ProjectId::ROOT;

        configuration.assign_slot(
            Some(root),
            SlotPosition { column: 1, row: 0 },
            SlotAssignment::Project {
                id: ProjectId::new(),
                name: "extra".into(),
            },
        );
        document.persist(&configuration);

        let text = fs::read_to_string(&path)?;
        assert!(
            text.contains(r#"project extra at="1,0""#),
            "unexpected document: {text}"
        );
        Ok(())
    }

    /// A persist reads the file again, so a comment added by hand while the terminal runs survives
    /// the next change.
    #[test]
    fn a_persist_keeps_the_comments_of_the_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.kdl");
        let mut configuration = parse_configuration(&path, r#"launcher shell at="0,0""#)?;
        fs::write(&path, "// The shell.\nlauncher shell at=\"0,0\"\n")?;

        configuration.assign_slot(
            Some(ProjectId::ROOT),
            SlotPosition { column: 1, row: 0 },
            SlotAssignment::Project {
                id: ProjectId::new(),
                name: "extra".into(),
            },
        );
        ConfigurationPersistence::new(&path).persist(&configuration);

        assert_eq!(
            fs::read_to_string(&path)?,
            "// The shell.\nlauncher shell at=\"0,0\"\nproject extra at=\"1,0\"\n"
        );
        Ok(())
    }

    /// A file that does not load is a hand edit in progress. A persist leaves it alone, even when
    /// it parses as KDL, because the reconcile would remove the unknown node.
    #[test]
    fn a_persist_leaves_a_file_that_does_not_load_alone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.kdl");
        let configuration = parse_configuration(&path, r#"launcher shell at="0,0""#)?;

        for text in [
            "launcher shell at=\"0,0\" {",
            "launcher shell at=\"0,0\"\nwindow 1\n",
        ] {
            fs::write(&path, text)?;
            ConfigurationPersistence::new(&path).persist(&configuration);
            assert_eq!(fs::read_to_string(&path)?, text);
        }
        Ok(())
    }
}
