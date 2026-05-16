//! A configuration derived hierarchy with assigned ids.
use anyhow::{Context, Result, anyhow};
use derive_more::{From, Into};
use uuid::Uuid;

use crate::projects::configuration::{LaunchProfile, ProjectConfiguration, ProjectSpec};

#[derive(Debug)]
pub struct ProjectSet {
    pub start: Option<LaunchProfileId>,
    pub projects: Vec<Project>,
}

#[derive(Debug)]
pub struct Project {
    pub id: ProjectId,
    pub properties: ProjectProperties,
    pub launchers: Vec<Launcher>,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct ProjectProperties {
    pub name: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MatrixPlacement {
    pub column: u32,
    pub row: u32,
}

#[derive(Debug)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub profile: LaunchProfile,
    pub placement: MatrixPlacement,
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

impl ProjectSet {
    pub fn from_configuration(config: ProjectConfiguration) -> Result<Self> {
        let projects: Vec<_> = config.projects.into_iter().map(convert_project).collect();

        let start = match config.startup {
            Some(profile_name) => Some(
                find_profile_by_name(&projects, &profile_name).with_context(|| {
                    format!(
                        "Startup profile '{}' not found in configuration",
                        profile_name
                    )
                })?,
            ),
            None => None,
        };

        Ok(Self { start, projects })
    }

    #[allow(unused)]
    pub fn get_launch_profile(&self, id: LaunchProfileId) -> Option<&LaunchProfile> {
        self.projects
            .iter()
            .find_map(|project| project.get_launch_profile(id))
    }
}

impl Project {
    fn get_launch_profile(&self, id: LaunchProfileId) -> Option<&LaunchProfile> {
        self.launchers
            .iter()
            .find(|launcher| launcher.id == id)
            .map(|launcher| &launcher.profile)
    }
}

fn find_profile_by_name(projects: &[Project], name: &str) -> Result<LaunchProfileId> {
    projects
        .iter()
        .flat_map(|project| &project.launchers)
        .find(|launcher| launcher.profile.name == name)
        .map(|launcher| launcher.id)
        .ok_or_else(|| anyhow!("Launch profile '{}' not found", name))
}

fn convert_project(project: ProjectSpec) -> Project {
    Project {
        id: ProjectId::new(),
        properties: ProjectProperties { name: project.name },
        launchers: project
            .launchers
            .into_iter()
            .map(|launcher| Launcher {
                id: LaunchProfileId::new(),
                profile: LaunchProfile {
                    name: launcher.name,
                    mode: launcher.mode,
                    tags: launcher.tags,
                    params: launcher.params,
                },
                placement: MatrixPlacement {
                    column: launcher.column,
                    row: launcher.row,
                },
            })
            .collect(),
    }
}
