use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpec {
    pub name: String,
    #[serde(default)]
    pub launchers: Vec<LauncherSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    #[serde(default)]
    pub mode: LauncherMode,
    #[allow(unused)]
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherSpec {
    pub name: String,
    pub column: u32,
    pub row: u32,
    #[serde(default)]
    pub mode: LauncherMode,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LauncherMode {
    Band,
    #[default]
    Visor,
}
