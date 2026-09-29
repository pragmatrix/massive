//! A project configuration represents ordered project sections with matrix-positioned launchers.

mod types;

pub use types::*;

#[derive(Debug)]
pub struct ProjectConfiguration {
    /// The startup profile.
    pub startup: Option<String>,
    pub projects: Vec<ProjectSpec>,
}
