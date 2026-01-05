use massive_shell::{ApplicationContext, Result};

use crate::{Application, application_registry::ApplicationRegistry, desktop::Desktop};

#[derive(Debug)]
pub struct DesktopEnvironment {
    pub primary_application: String,
    pub applications: ApplicationRegistry,
}

impl DesktopEnvironment {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            primary_application: applications
                .first()
                .expect("No primary application")
                .name
                .clone(),
            applications: ApplicationRegistry::new(applications),
        }
    }

    pub async fn run_desktop(self, context: ApplicationContext) -> Result<()> {
        let state = Desktop::new(self, context).await?;
        state.run().await
    }
}
