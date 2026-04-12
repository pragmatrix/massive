use serde::Deserialize;

use super::types::{LaunchGroupSpec, LayoutDirection};

/// Intermediate representation for deserializing JSON configuration files.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    /// The startup launch profile.
    #[serde(default)]
    pub startup: Option<String>,

    /// The root launch group.
    #[serde(default)]
    pub root: Option<LaunchGroupSpec>,
}

impl ConfigFile {
    pub fn into_launch_group(self, root_group_name: String) -> LaunchGroupSpec {
        self.root.unwrap_or(LaunchGroupSpec {
            name: root_group_name,
            layout: LayoutDirection::Horizontal,
            children: Vec::new(),
        })
    }
}
