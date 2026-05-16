//! A project configuration represents ordered project sections with matrix-positioned launchers.

pub mod json_reader;
mod types;

use anyhow::{Context, Result};
pub use types::*;

use json_reader::ConfigFile;

#[derive(Debug)]
pub struct ProjectConfiguration {
    /// The startup profile.
    pub startup: Option<String>,
    pub projects: Vec<ProjectSpec>,
}

impl ProjectConfiguration {
    /// Load a configuration file from JSON.
    pub fn from_json(json: &str, name: &str) -> Result<Self> {
        let config: ConfigFile = serde_json::from_str(json)
            .with_context(|| format!("Failed to parse JSON configuration {name}"))?;

        let startup = config.startup.clone();
        Ok(ProjectConfiguration {
            startup,
            projects: config.projects,
        })
    }
}
