use std::{collections::HashMap, future::Future, pin::Pin};

use derive_more::Debug;
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};
use uuid::Uuid;

use massive_applications::{
    ApplicationContext, ApplicationEvent, ApplicationId, ApplicationRequest,
};
use massive_shell::{Result, ShellContext};

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

    pub async fn run(mut self, mut context: ShellContext) -> Result<()> {
        let (requests_tx, mut requests_rx) =
            unbounded_channel::<(ApplicationId, ApplicationRequest)>();

        let mut event_senders: HashMap<ApplicationId, UnboundedSender<ApplicationEvent>> =
            HashMap::new();

        let mut join_set = JoinSet::new();

        for (app_id, app) in self.applications.drain() {
            let (events_tx, events_rx) = unbounded_channel::<ApplicationEvent>();
            event_senders.insert(app_id, events_tx);

            let app_context = ApplicationContext::new(app_id, requests_tx.clone(), events_rx);
            join_set.spawn((app.run)(app_context));
        }

        drop(requests_tx);

        loop {
            tokio::select! {
                Some((app_id, request)) = requests_rx.recv() => {
                    // TODO: Process ApplicationRequest variants
                    eprintln!("Received request from app {:?}: {:?}", app_id, request);
                }

                shell_event = context.wait_for_shell_event() => {
                    let _event = shell_event?;
                    // TODO: Process ShellEvent variants
                }

                Some(result) = join_set.join_next() => {
                    let app_result = result.unwrap_or_else(|e| Err(anyhow::anyhow!("Application failed: {}", e)));
                    return app_result;
                }

                else => {
                    return Ok(());
                }
            }
        }
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
