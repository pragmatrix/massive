use std::pin::Pin;
use std::{collections::HashMap, future::Future};

use derive_more::Debug;
use massive_applications::{ApplicationContext, ApplicationId};
use massive_shell::{Result, ShellContext};
use uuid::Uuid;

#[derive(Debug)]
pub struct Desktop {
    applications: HashMap<ApplicationId, Application>,
}

impl Desktop {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.id, a))),
        }
    }

    pub async fn run(_context: ShellContext) -> Result<()> {
        // TODO: Wire up application lifecycle and shell integration.
        Ok(())
    }
}

#[derive(Debug)]
struct Application {
    name: String,
    /// This is the process local id for the application (not visible to the outside for now)
    id: ApplicationId,
    #[debug(skip)]
    run: ApplicationRunBox,
}

type ApplicationRunBox = Box<
    dyn Fn(ApplicationContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

impl Application {
    pub fn new<F, R>(name: impl Into<String>, run: F) -> Self
    where
        F: Fn(ApplicationContext) -> R + Send + Sync + 'static,
        R: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let run_boxed = Box::new(
            move |ctx: ApplicationContext| -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                Box::pin(run(ctx))
            },
        );

        Self {
            id: Uuid::new_v4().into(),
            name,
            run: run_boxed,
        }
    }
}

// Debug derived via derive_more with run skipped
