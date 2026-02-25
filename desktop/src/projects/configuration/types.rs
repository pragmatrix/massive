use serde_json::{Map, Value};

use massive_layout::LayoutAxis;

#[derive(Debug)]
pub struct LaunchGroupSpec {
    pub name: String,
    pub tag: ScopedTag,
    pub layout: LayoutDirection,
    pub content: GroupContents,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LayoutDirection {
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

/// A group can only contain either groups or applications.
#[derive(Debug)]
pub enum GroupContents {
    Groups(Vec<LaunchGroupSpec>),
    Profiles(Vec<LaunchProfile>),
}

#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub mode: LauncherMode,
    pub params: Map<String, Value>,
    pub tags: Vec<ScopedTag>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LauncherMode {
    Band,
    Visor,
}

#[derive(Debug, Clone)]
pub struct ScopedTag {
    pub scope: String,
    pub tag: String,
}

impl ScopedTag {
    pub fn new(scope: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            tag: tag.into(),
        }
    }
}
