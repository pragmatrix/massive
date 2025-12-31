#[derive(Debug)]
pub struct ProjectConfiguration {
    pub name: String,
    pub groups: Vec<ApplicationGroup>,
}

#[derive(Debug)]
pub struct ApplicationGroup {
    pub name: String,
    pub tag: ScopedTag,
    pub direction: LayoutDirection,
    pub nested: Vec<ApplicationGroup>,
}

#[derive(Debug)]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
}

/// A group can only contain either groups or applications.
#[derive(Debug)]
pub enum GroupContents {
    Groups(Vec<ApplicationGroup>),
    Applications(Vec<ApplicationRef>),
}

#[derive(Debug, Clone)]
pub struct ScopedTag {
    pub scope: String,
    pub tag: String,
}

impl ScopedTag {
    pub fn new(scope: String, tag: String) -> Self {
        Self { scope, tag }
    }
}

#[derive(Debug)]
pub struct ApplicationRef {
    pub name: String,
    pub params: Parameters,
    pub tags: Vec<ScopedTag>,
}

#[derive(Debug, derive_more::Deref)]
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