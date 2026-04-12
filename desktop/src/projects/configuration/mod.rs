//! A project configuration represents a plane in space that has a specific layout of tiles and
//! groups and acts as a launching space for new applications.

pub mod json_reader;
mod types;

use anyhow::{Context, Result};
pub use types::*;

use json_reader::ConfigFile;

#[derive(Debug)]
pub struct ProjectConfiguration {
    /// The startup profile.
    pub startup: Option<String>,
    pub root: LaunchGroupSpec,
}

impl ProjectConfiguration {
    /// Load a configuration file from JSON. `name` denotes the name of the root group of this
    /// configuration.
    pub fn from_json(json: &str, root_group_name: &str) -> Result<Self> {
        let config: ConfigFile = serde_json::from_str(json)
            .with_context(|| format!("Failed to parse JSON configuration {root_group_name}"))?;

        let startup = config.startup.clone();
        let group = config.into_launch_group(root_group_name.into());
        Ok(ProjectConfiguration {
            startup,
            root: group,
        })
    }
}
