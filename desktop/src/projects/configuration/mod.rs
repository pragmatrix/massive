//! A project configuration represents a plane in space that has a specific layout of tiles and
//! groups and acts as a launching space for new applications.

pub mod toml_reader;
mod types;

use anyhow::{Context, Result};
pub use types::*;

use toml_reader::ConfigFile;

#[derive(Debug)]
pub struct ProjectConfiguration {
    /// The startup profile.
    pub startup: Option<String>,
    pub root: LaunchGroup,
}

impl ProjectConfiguration {
    /// Load a configuration file from a TOML file. `name` denotes the name of the root group of this
    /// configuration.
    pub fn from_toml(toml: &str, root_group_name: &str) -> Result<Self> {
        let config: ConfigFile = toml::from_str(&toml)
            .with_context(|| format!("Failed to parse TOML configuration {root_group_name}"))?;

        let startup = config.startup.clone();

        let group = config.into_launch_group(root_group_name.into())?;
        Ok(ProjectConfiguration {
            startup,
            root: group,
        })
    }
}
