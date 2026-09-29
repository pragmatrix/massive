//! The desktop configuration aggregate: projects with matrix-placed launchers,
//! each launcher a spawnable profile, and the profile the session boots into.

use anyhow::{Context, Result, ensure};
use derive_more::{From, Into};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Spawn parameters of a launcher: JSON values passed to the spawned application.
pub type Params = Map<String, Value>;

/// The canonical desktop configuration: what is parsed from KDL and what the boot
/// flow derives its commands from. Ids are fresh per session and the document is
/// keyed by name, so the two representations only meet during parse and persist.
#[derive(Debug)]
pub struct DesktopConfiguration {
    /// Document order is preserved, so boot commands build the scene like the
    /// document reads.
    projects: Vec<Project>,
    startup: Option<LaunchProfileId>,
}

impl DesktopConfiguration {
    /// Builds the aggregate from parsed projects and the named startup profile.
    ///
    /// Fails when no launcher exists at all: the session boots into a
    /// configuration-defined launcher, so an empty configuration has nothing to
    /// start.
    pub(super) fn new(projects: Vec<Project>, startup: Option<&str>) -> Result<Self> {
        let empty = projects.iter().all(|project| project.launchers.is_empty());
        ensure!(!empty, "Configuration must define at least one launcher");

        let startup = match startup {
            Some(name) => {
                let id = find_launcher_by_name(&projects, name).with_context(|| {
                    format!("Startup profile '{name}' not found in configuration")
                })?;
                Some(id)
            }
            None => None,
        };

        Ok(Self { projects, startup })
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn startup(&self) -> Option<LaunchProfileId> {
        self.startup
    }
}

/// Resolves the startup profile's id by name among all launchers.
fn find_launcher_by_name(projects: &[Project], name: &str) -> Option<LaunchProfileId> {
    projects
        .iter()
        .flat_map(|project| project.launchers.iter())
        .find(|launcher| launcher.name == name)
        .map(|launcher| launcher.id)
}

#[derive(Debug)]
pub struct Project {
    id: ProjectId,
    name: String,
    launchers: Vec<Launcher>,
}

impl Project {
    /// Mints the project's id and puts its launchers into sorted order.
    ///
    /// The sort is stable, so launchers sharing a placement keep their document
    /// order.
    pub(crate) fn new(name: String, launchers: Vec<Launcher>) -> Self {
        // Invariant: launchers stay sorted by matrix placement, so consumers can
        // rely on order without re-reading placements.
        let mut launchers = launchers;
        launchers.sort_by_key(|launcher| (launcher.placement.column, launcher.placement.row));
        Self {
            id: ProjectId::new(),
            name,
            launchers,
        }
    }

    pub fn id(&self) -> ProjectId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn launchers(&self) -> &[Launcher] {
        &self.launchers
    }
}

#[derive(Debug)]
pub struct Launcher {
    id: LaunchProfileId,
    name: String,
    mode: LauncherMode,
    params: Params,
    placement: MatrixPlacement,
}

impl Launcher {
    /// Mints the launcher's id.
    pub(crate) fn new(
        name: String,
        mode: LauncherMode,
        params: Params,
        placement: MatrixPlacement,
    ) -> Self {
        Self {
            id: LaunchProfileId::new(),
            name,
            mode,
            params,
            placement,
        }
    }

    pub fn id(&self) -> LaunchProfileId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    // Part of the aggregate's query API; consumed as mutation moves off parse in
    // later plan steps.
    #[allow(unused)]
    pub fn mode(&self) -> LauncherMode {
        self.mode
    }

    #[allow(unused)]
    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn placement(&self) -> MatrixPlacement {
        self.placement
    }

    /// The launcher's profile as carried by `AddLauncher` changes.
    pub fn profile(&self) -> LaunchProfile {
        LaunchProfile {
            name: self.name.clone(),
            mode: self.mode,
            params: self.params.clone(),
        }
    }
}

/// The spawnable profile of a launcher, carried by changes and shown by the
/// `LauncherPresenter`.
#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub mode: LauncherMode,
    pub params: Params,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MatrixPlacement {
    pub column: u32,
    pub row: u32,
}

impl From<(u32, u32)> for MatrixPlacement {
    fn from((column, row): (u32, u32)) -> Self {
        Self { column, row }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct ProjectId(Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct LaunchProfileId(Uuid);

impl LaunchProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum LauncherMode {
    Band,
    #[default]
    Visor,
}
