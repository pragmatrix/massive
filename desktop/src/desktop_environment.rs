use std::{
    env,
    path::{Path, PathBuf},
};

use massive_shell::{ApplicationContext, Result};

use crate::{Application, application_registry::ApplicationRegistry, desktop::Desktop};

#[derive(Debug)]
pub struct DesktopEnvironment {
    pub primary_application: String,
    pub applications: ApplicationRegistry,
    /// The directory pointing to the project directory. This is usually a subdirectory under the
    /// user's home directory.
    ///
    /// Default is the home directory of the current user / ".massive".
    pub projects_dir: Option<PathBuf>,
}

const DEFAULT_PROJECT_DIR: &str = ".massive";

impl DesktopEnvironment {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            primary_application: applications
                .first()
                .expect("No primary application")
                .name
                .clone(),
            applications: ApplicationRegistry::new(applications),
            projects_dir: None,
        }
    }

    /// Returns either the set projects dir or the default one if a home directory can be resolved.
    pub fn final_projects_dir(&self) -> Option<PathBuf> {
        self.projects_dir
            .clone()
            .or_else(|| env::home_dir().map(|hd| hd.join(DEFAULT_PROJECT_DIR)))
    }

    pub fn with_projects(mut self, projects_dir: impl AsRef<Path>) -> Self {
        self.projects_dir = Some(projects_dir.as_ref().into());
        self
    }

    pub async fn run_desktop(self, context: ApplicationContext) -> Result<()> {
        let state = Desktop::new(self, context).await?;
        state.run().await
    }
}
