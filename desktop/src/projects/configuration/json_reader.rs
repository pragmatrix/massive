use serde::Deserialize;

use super::types::ProjectSpec;

/// Intermediate representation for deserializing JSON configuration files.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    /// The startup launch profile.
    #[serde(default)]
    pub startup: Option<String>,

    /// The ordered project sections.
    #[serde(default)]
    pub projects: Vec<ProjectSpec>,
}
