use derive_more::Deref;

#[derive(Debug)]
pub struct LaunchGroup {
    pub name: String,
    pub tag: ScopedTag,
    pub direction: LayoutDirection,
    pub content: GroupContents,
}

#[derive(Debug)]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
}

/// A group can only contain either groups or applications.
#[derive(Debug)]
pub enum GroupContents {
    Groups(Vec<LaunchGroup>),
    LaunchProfiles(Vec<LaunchProfile>),
}

#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub params: Parameters,
    pub tags: Vec<ScopedTag>,
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

#[derive(Debug, Deref, Clone)]
pub struct Parameters(pub Vec<Parameter>);

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub value: String,
}
impl Parameter {
    pub fn new(name: String, value: String) -> Self {
        Self { name, value }
    }
}
