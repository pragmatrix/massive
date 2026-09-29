//! The desktop configuration document: the parsed KDL representation plus the
//! id → name map, changed together and persisted atomically at the end of a
//! transaction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use log::debug;

use kdl::KdlDocument;

use crate::desktop_system::change::ProjectChange;
use crate::projects::{
    LaunchProfileId, LauncherMode, MatrixPlacement, ProjectConfiguration, ProjectId, ProjectSet,
};

/// The desktop configuration: the parsed KDL document plus the id → name map that
/// resolves live-model ids into the document's vocabulary.
///
/// Changes land in memory immediately. The file is written synchronously and
/// atomically, once per transaction, so a transaction's several changes persist as
/// a single write.
#[derive(Debug)]
pub struct ConfigurationDocument {
    document: KdlDocument,
    /// Where the configuration lives. Created at load time: the default
    /// configuration is written when no file exists yet, so every later change has
    /// a file to be persisted to.
    path: PathBuf,
    /// Whether applied configuration changes have not yet been written to the file
    /// on disk.
    unwritten_changes: bool,
    /// Maps configuration ids to the document's names for the persistence edits.
    keys: ConfigKeys,
}

impl ConfigurationDocument {
    /// Loads the configuration from the projects directory, falling back to the
    /// built-in default — which is written immediately, so the file always exists
    /// on disk before the first change is persisted.
    ///
    /// A file that exists but cannot be read or parsed aborts startup.
    pub fn load(projects_dir: &Path) -> Result<Self> {
        let path = projects_dir.join(super::kdl_codec::CONFIG_FILE_NAME);
        let document = match std::fs::read_to_string(&path) {
            Ok(text) => text
                .parse()
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    "No configuration at {}, writing the default configuration",
                    path.display()
                );
                let document = super::kdl_codec::default_document();
                super::kdl_codec::atomic_write(&path, &document.to_string()).with_context(
                    || format!("writing the default configuration to {}", path.display()),
                )?;
                document
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };

        Ok(Self {
            document,
            path,
            unwritten_changes: false,
            keys: ConfigKeys::default(),
        })
    }

    /// The configuration, resolved from the document.
    pub fn configuration(&self) -> Result<ProjectConfiguration> {
        super::kdl_codec::configuration_from_document(&self.document)
    }

    /// Registers the names of the projects and launchers loaded from the
    /// configuration file.
    ///
    /// The configuration's order matches the document's, so the loaded names are
    /// already unique; registration cannot introduce a collision.
    pub fn register_loaded(&mut self, project_set: &ProjectSet) {
        self.keys.register_loaded(project_set);
    }

    /// Applies a configuration change to the in-memory document.
    ///
    /// A failed edit is returned without changing the document, so callers can
    /// report that persistence was rejected.
    pub fn apply(&mut self, change: ProjectChange) -> Result<()> {
        let change = self.keys.translate_change(&change)?;
        super::kdl_codec::apply_change(&mut self.document, &change)?;
        self.unwritten_changes = true;
        Ok(())
    }

    /// Writes the document once if the in-memory document differs from the file on
    /// disk.
    ///
    /// Called at the end of a transaction, so its several changes persist as one
    /// file write. A write failure is logged and leaves `unwritten_changes` set, so
    /// the next flush retries it.
    pub fn flush(&mut self) {
        if !self.unwritten_changes {
            return;
        }
        let text = self.document.to_string();
        match super::kdl_codec::atomic_write(&self.path, &text) {
            Ok(()) => self.unwritten_changes = false,
            Err(error) => log::warn!(
                "Failed to persist configuration to {}: {error:#}",
                self.path.display()
            ),
        }
    }
}

/// Maps configuration ids to document names across a session.
///
/// The document is keyed by name, and launchers get fresh ids every session, so every
/// applied change needs its id's name resolved. Names must be unique among sibling
/// nodes: registration dedupes by appending ` 2`, ` 3`, ... and keeps the final name,
/// so the live model and the document stay aligned.
#[derive(Debug, Clone, Default)]
struct ConfigKeys {
    projects: HashMap<ProjectId, String>,
    launchers: HashMap<LaunchProfileId, (ProjectId, String)>,
}

impl ConfigKeys {
    /// Registers the names of projects and launchers loaded from the file.
    fn register_loaded(&mut self, project_set: &ProjectSet) {
        for project in &project_set.projects {
            self.projects
                .insert(project.id, project.properties.name.clone());
            for launcher in &project.launchers {
                self.launchers
                    .insert(launcher.id, (project.id, launcher.profile.name.clone()));
            }
        }
    }

    /// Registers a project under a document-unique name, renaming it on collision.
    fn register_project(&mut self, id: ProjectId, name: &str) -> Result<String> {
        let name = dedupe(name, self.projects.values().map(String::as_str));
        self.projects.insert(id, name.clone());
        Ok(name)
    }

    /// Registers a launcher under a name unique among its project's siblings,
    /// renaming it on collision. Returns the launcher's owning project's (possibly
    /// renamed) and the launcher's final name.
    fn register_launcher(
        &mut self,
        project: ProjectId,
        id: LaunchProfileId,
        name: &str,
    ) -> Result<(String, String)> {
        let project_name = self
            .projects
            .get(&project)
            .with_context(|| "adding a launcher to an unregistered project")?
            .clone();
        let name = dedupe(
            name,
            self.launchers
                .values()
                .filter(move |(owner, _)| *owner == project)
                .map(|(_, name)| name.as_str()),
        );
        self.launchers.insert(id, (project, name.clone()));
        Ok((project_name, name))
    }

    /// Resolves a launcher's owning project's name and its own name.
    fn launcher_key(&self, id: &LaunchProfileId) -> Result<(&str, &str)> {
        let (project, name) = self
            .launchers
            .get(id)
            .with_context(|| format!("launcher {id:?} was never registered for persistence"))?;
        let project_name = self
            .projects
            .get(project)
            .with_context(|| "launcher's owning project was never registered for persistence")?;
        Ok((project_name.as_str(), name.as_str()))
    }

    /// Drops a launcher's registration, returning its owning project's name and its
    /// own name.
    fn remove_launcher(&mut self, id: &LaunchProfileId) -> Result<(String, String)> {
        let (project, name) = self
            .launchers
            .remove(id)
            .with_context(|| format!("launcher {id:?} was never registered for persistence"))?;
        let project_name = self
            .projects
            .get(&project)
            .with_context(|| "launcher's owning project was never registered for persistence")?
            .clone();
        Ok((project_name, name))
    }

    /// Drops a project's and its launchers' registrations, returning the project's
    /// name.
    fn remove_project(&mut self, id: &ProjectId) -> Result<String> {
        self.launchers.retain(|_, (project, _)| project != id);
        self.projects
            .get(id)
            .with_context(|| format!("project {id:?} was never registered for persistence"))
            .cloned()
    }

    /// Translates a configuration change from live-model ids into document terms.
    fn translate_change(&mut self, change: &ProjectChange) -> Result<ConfigChange> {
        match change {
            ProjectChange::AddProject { id, properties } => {
                let name = self.register_project(*id, &properties.name)?;
                Ok(ConfigChange::AddProject { name })
            }
            ProjectChange::RemoveProject(project) => {
                let name = self.remove_project(project)?;
                Ok(ConfigChange::RemoveProject { name })
            }
            ProjectChange::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let (project, name) = self.register_launcher(*project, *id, &profile.name)?;
                Ok(ConfigChange::AddLauncher {
                    project,
                    name,
                    mode: profile.mode,
                    params: profile.params.clone(),
                    placement: *placement,
                })
            }
            ProjectChange::MoveLauncher {
                launcher,
                placement,
            } => {
                let (project, name) = self.launcher_key(launcher)?;
                Ok(ConfigChange::MoveLauncher {
                    project: project.into(),
                    name: name.into(),
                    placement: *placement,
                })
            }
            ProjectChange::RemoveLauncher(launch_profile_id) => {
                let (project, name) = self.remove_launcher(launch_profile_id)?;
                Ok(ConfigChange::RemoveLauncher { project, name })
            }
            ProjectChange::SetStartupProfile(id) => {
                // `None` clears the startup profile: the `startup` node is removed
                // from the document.
                let name = match id {
                    Some(id) => Some(self.launcher_key(id)?.1.into()),
                    None => None,
                };
                Ok(ConfigChange::SetStartup(name))
            }
        }
    }
}

/// Appends a numeric suffix while a name collides with any existing name.
fn dedupe<'a>(name: &str, existing: impl Iterator<Item = &'a str> + Clone) -> String {
    if !existing.clone().any(|existing| existing == name) {
        return name.into();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{name} {suffix}");
        if !existing.clone().any(|existing| existing == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// A configuration change expressed in document terms: names instead of ids.
#[derive(Debug, Clone)]
pub(super) enum ConfigChange {
    SetStartup(Option<String>),
    AddProject {
        name: String,
    },
    RemoveProject {
        name: String,
    },
    AddLauncher {
        project: String,
        name: String,
        mode: LauncherMode,
        params: serde_json::Map<String, serde_json::Value>,
        placement: MatrixPlacement,
    },
    MoveLauncher {
        project: String,
        name: String,
        placement: MatrixPlacement,
    },
    RemoveLauncher {
        project: String,
        name: String,
    },
}
