use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct ProjectSpec {
    pub name: String,
    pub launchers: Vec<LauncherSpec>,
}

#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub mode: LauncherMode,
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct LauncherSpec {
    pub name: String,
    pub column: u32,
    pub row: u32,
    pub mode: LauncherMode,
    pub params: Map<String, Value>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum LauncherMode {
    Band,
    #[default]
    Visor,
}
