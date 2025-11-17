use std::{collections::HashMap, future::Future, pin::Pin};

use derive_more::Debug;
use tokio::{sync::mpsc::unbounded_channel, task::JoinSet};
use uuid::Uuid;
use winit::dpi::LogicalSize;

use massive_applications::{CreationMode, InstanceContext, InstanceId, InstanceRequest};
use massive_shell::{Result, Scene, ShellContext};

#[derive(Debug)]
pub struct Desktop {
    applications: HashMap<String, Application>,
}

impl Desktop {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.name.clone(), a))),
        }
    }

    pub async fn run(self, mut context: ShellContext) -> Result<()> {
        // Create a window and renderer
        let window = context.new_window(LogicalSize::new(1024, 768)).await?;
        let _renderer = window.renderer().build().await?;
        let _scene = Scene::new();

        let (requests_tx, mut requests_rx) = unbounded_channel::<(InstanceId, InstanceRequest)>();

        let mut join_set = JoinSet::new();

        // Start one instance of the first registered application
        if let Some(app) = self.applications.values().next() {
            let instance_id = InstanceId::from(Uuid::new_v4());
            let (_events_tx, events_rx) = unbounded_channel();

            let instance_context = InstanceContext::new(
                instance_id,
                CreationMode::New,
                requests_tx.clone(),
                events_rx,
            );

            join_set.spawn((app.run)(instance_context));
        }

        drop(requests_tx);

        loop {
            tokio::select! {
                Some((instance_id, request)) = requests_rx.recv() => {
                    // TODO: Process InstanceRequest variants
                    eprintln!("Received request from instance {:?}: {:?}", instance_id, request);
                }

                shell_event = context.wait_for_shell_event() => {
                    let _event = shell_event?;
                    // TODO: Process ShellEvent variants
                }

                Some(result) = join_set.join_next() => {
                    let instance_result = result.unwrap_or_else(|e| Err(anyhow::anyhow!("Instance ended with error: {}", e)));
                    return instance_result;
                }

                else => {
                    return Ok(());
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Application {
    name: String,
    #[debug(skip)]
    run: RunInstanceBox,
}

type RunInstanceBox = Box<
    dyn Fn(InstanceContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

impl Application {
    pub fn new<F, R>(name: impl Into<String>, run: F) -> Self
    where
        F: Fn(InstanceContext) -> R + Send + Sync + 'static,
        R: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let run_boxed = Box::new(
            move |ctx: InstanceContext| -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                Box::pin(run(ctx))
            },
        );

        Self {
            name,
            run: run_boxed,
        }
    }
}
