use serde::Deserialize;
use serde_json::{Map, Value};

use massive_layout::LayoutAxis;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchGroupSpec {
    pub name: String,
    #[serde(default)]
    pub layout: LayoutDirection,
    #[serde(default)]
    pub children: Vec<GroupChildSpec>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayoutDirection {
    #[default]
    Horizontal,
    Vertical,
}

impl LayoutDirection {
    pub fn axis(&self) -> LayoutAxis {
        match self {
            Self::Horizontal => LayoutAxis::HORIZONTAL,
            Self::Vertical => LayoutAxis::VERTICAL,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GroupChildSpec {
    Group {
        name: String,
        #[serde(default)]
        layout: LayoutDirection,
        #[serde(default)]
        children: Vec<GroupChildSpec>,
    },
    Launcher {
        name: String,
        #[serde(default)]
        mode: LauncherMode,
        #[serde(default)]
        params: Map<String, Value>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    #[serde(default)]
    pub mode: LauncherMode,
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
