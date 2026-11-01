use std::env;
use std::path::{Path, PathBuf};

use anyhow::bail;
use log::error;
use massive_shell::{ApplicationContext, Result};

use crate::tools::ToolCatalog;
use massive_applications::Application;

use crate::{application_registry::ApplicationRegistry, desktop::Desktop};

#[derive(Debug)]
pub struct DesktopEnvironment {
    pub primary_application: String,
    pub(crate) applications: ApplicationRegistry,
    /// Every tool agents can call, built from `applications` (ADR 0036).
    pub(crate) tool_catalog: ToolCatalog,
    /// The directory pointing to the project directory. This is usually a subdirectory under the
    /// user's home directory.
    ///
    /// Default is the home directory of the current user / ".massive".
    pub projects_dir: Option<PathBuf>,
}

const DEFAULT_PROJECT_DIR: &str = ".massive";

impl DesktopEnvironment {
    /// Fails without applications, because the first one is the primary application, and when the
    /// [`ApplicationRegistry`] or the [`ToolCatalog`] refuses them. Every mistake in the
    /// applications' declarations therefore shows before the desktop starts.
    pub fn new(applications: Vec<Application>) -> Result<Self> {
        let Some(primary) = applications.first() else {
            bail!("the desktop needs at least one application, the primary one");
        };
        let primary_application = primary.name().to_owned();
        let applications = ApplicationRegistry::new(applications)?;
        Ok(Self {
            primary_application,
            tool_catalog: ToolCatalog::new(&applications)?,
            applications,
            projects_dir: None,
        })
    }

    /// Returns either the set projects dir or the default one if a home directory can be resolved.
    pub fn projects_dir(&self) -> Option<PathBuf> {
        self.projects_dir
            .clone()
            .or_else(|| env::home_dir().map(|hd| hd.join(DEFAULT_PROJECT_DIR)))
    }

    pub fn with_projects(mut self, projects_dir: impl AsRef<Path>) -> Self {
        self.projects_dir = Some(projects_dir.as_ref().into());
        self
    }

    pub async fn run_desktop(self, context: ApplicationContext) -> Result<()> {
        let mut desktop = Desktop::new(self, context).await?;
        let r = desktop.run().await;
        // We log the error here, so that when we crash in Desktop drop, the original error does not get lost.
        if let Err(e) = &r {
            error!("Desktop crashed: {e:?}");
        }
        r
    }
}
