use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result};
use derive_more::From;
use log::warn;

use crate::{
    band_presenter::BandTarget,
    projects::configuration::{GroupContents, LaunchProfile, LayoutDirection, ScopedTag},
};

mod configuration;
mod launcher_presenter;
mod project;
mod project_presenter;

pub use configuration::ProjectConfiguration;
pub use launcher_presenter::LauncherPresenter;
pub use project::{GroupId, LaunchProfileId, Project};
pub use project_presenter::ProjectPresenter;

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq, From)]
pub enum ProjectTarget {
    Group(GroupId),
    Launcher(LaunchProfileId),
    // Under Launcher
    // Architecture: Why do we need to have the LaunchProfileId here for navigating down?
    Band(LaunchProfileId, BandTarget),
}

impl ProjectConfiguration {
    /// Loads the configuration from the the project directory. If the project directory is not set,
    /// or if the file "desktop.toml" is not found, falls back to the default configuration.
    pub fn from_dir(projects_dir: Option<&Path>) -> Result<Self> {
        let Some(projects_dir) = projects_dir else {
            return Ok(Self::default());
        };

        const DESKTOP_CONFIG: &str = "desktop";

        let path = projects_dir.join(format!("{DESKTOP_CONFIG}.toml"));

        if !fs::exists(&path)? {
            warn!(
                "Configuration file not found, falling back to default configuration: {}",
                path.display()
            );
            return Ok(Self::default());
        }

        let toml = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read toml file: {}", path.display()))?;

        ProjectConfiguration::from_toml(&toml, DESKTOP_CONFIG)
    }
}

impl Default for ProjectConfiguration {
    fn default() -> Self {
        const DEFAULT_PROFILE: &str = "default";

        ProjectConfiguration {
            startup: Some(DEFAULT_PROFILE.into()),
            root: configuration::LaunchGroup {
                name: "/".into(),
                tag: ScopedTag::new("", ""),
                layout: LayoutDirection::Horizontal,
                content: GroupContents::Profiles(
                    [LaunchProfile {
                        name: DEFAULT_PROFILE.into(),
                        params: Default::default(),
                        tags: Vec::new(),
                    }]
                    .into(),
                ),
            },
        }
    }
}
