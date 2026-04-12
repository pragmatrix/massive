use derive_more::{Deref, Display, From, Into};

#[derive(Debug)]
pub enum ProjectCommand {
    AddLauncher {
        name: LauncherName,
        group: Option<LaunchGroupName>,
    },
    RemoveLauncher(LauncherName),
    AddToGroup {
        parent: Option<LaunchGroupName>,
        group: LaunchGroupName,
        properties: LaunchGroupProperties,
    },
    RemoveFromGroup(Option<LaunchGroupName>),
    SetStartupProfile(Option<LauncherName>),
}

#[derive(Debug, Clone)]
pub struct LaunchGroupProperties {
    pub layout: LayoutDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, From, Into, Deref, Display)]
pub struct LaunchGroupName(String);

impl LaunchGroupName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, From, Into, Deref, Display)]
pub struct LauncherName(String);

impl LauncherName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
}
