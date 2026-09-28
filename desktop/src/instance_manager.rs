use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::thread;

use anyhow::{Context, anyhow, bail};
use derive_more::{Debug, From, Into};
use futures::FutureExt;
use log::warn;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::task_context::{self, TaskContext};
use massive_applications::{
    ApplicationMessage, CreationMode, InstanceChange, InstanceContext, InstanceEnvironment,
    InstanceId, ViewEvent, ViewId,
};
use massive_scene::AnyCollector;
use massive_scene::{Location, Ref};
use massive_shell::Result;

use crate::application_registry::{Application, RuntimeKind};

/// Manages running application instances with lifecycle control.
///
/// Each instance runs on its own dedicated OS thread with its own runtime, detached from
/// the desktop's shared worker pool (ADR 0009). Completions arrive on the mpsc channel;
/// the threads are never joined or aborted — shutdown is cooperative and the process end
/// reaps threads that outlive the shutdown deadline.
#[derive(Debug)]
pub struct InstanceManager {
    instances: HashMap<InstanceId, RunningInstance>,
    environment: InstanceEnvironment,
    /// Sender the instance threads report their result on. Held by the manager so the
    /// receiver in `completions_rx` stays open between spawns.
    completions_tx: UnboundedSender<(InstanceId, Result<()>)>,
    completions_rx: UnboundedReceiver<(InstanceId, Result<()>)>,
}

#[derive(Debug)]
struct RunningInstance {
    #[allow(unused)]
    application_name: String,
    events_tx: UnboundedSender<ApplicationMessage>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, From, Into)]
pub struct ViewPath {
    pub instance: InstanceId,
    pub view: ViewId,
}

impl InstanceManager {
    pub fn new(environment: InstanceEnvironment) -> Self {
        let (completions_tx, completions_rx) = unbounded_channel();
        Self {
            environment,
            instances: HashMap::new(),
            completions_tx,
            completions_rx,
        }
    }

    /// Spawn a new instance of an application with a pre-created [`InstanceId`].
    pub fn spawn(
        &mut self,
        instance_id: InstanceId,
        application: &Application,
        creation_mode: CreationMode,
        root: Ref<Location>,
    ) -> Result<()> {
        let (events_tx, events_rx) = unbounded_channel();
        let environment = self.environment.clone();
        let instance_context =
            InstanceContext::new(instance_id, creation_mode, environment, root, events_rx);
        let instance_future = (application.run)(instance_context);
        let runtime_kind = application.runtime_kind;
        // Own coordinator per instance: a timestamp from one instance must not affect another's
        // animations when instances run in parallel. The shaping context is a fresh scratch over
        // the manager the task context reaches, so instances shape in parallel on their own
        // scratch (ADR 0006) — and it must be created here, on the desktop task: the instance
        // thread runs outside the desktop's task-locals (ADR 0008/0009). ShapingContext is
        // Send, so the move into the thread closure is sound.
        let instance_task_context = TaskContext::new(
            AnyCollector::for_type::<InstanceChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            task_context::fonts().new_shaping_context(),
        );
        let completions_tx = self.completions_tx.clone();
        // Detached thread: the manager never joins or aborts it (ADR 0009).
        spawn_instance_thread(
            instance_id,
            runtime_kind,
            instance_future,
            instance_task_context,
            completions_tx,
        )
        .with_context(|| format!("Spawning instance thread for {instance_id:?}"))?;

        self.instances.insert(
            instance_id,
            RunningInstance {
                application_name: application.name.clone(),
                events_tx,
            },
        );

        Ok(())
    }

    pub fn request_shutdown_all(&self) -> Result<()> {
        for instance_id in self.instances.keys().copied() {
            if let Err(error) = self.request_shutdown(instance_id) {
                warn!("Instance {instance_id:?} ended before shutdown: {error}");
            }
        }
        Ok(())
    }

    /// Begin the shutdown of an instance by sending [`ApplicationMessage::Shutdown`]. Returns
    /// immediately after sending the event
    pub fn request_shutdown(&self, instance_id: InstanceId) -> Result<()> {
        let instance = self.instances.get(&instance_id).ok_or_else(|| {
            anyhow!(
                "Failed to request a shutdown: Instance {:?} not found",
                instance_id
            )
        })?;

        instance
            .events_tx
            .send(ApplicationMessage::Shutdown(instance_id))
            .map_err(|_| {
                anyhow!(
                    "Failed to send shutdown event to instance {:?}",
                    instance_id
                )
            })
    }

    /// Wait for the next instance to complete and handle cleanup.
    ///
    /// Returns an error when no instances are running, or when the completion
    /// channel closed because the manager was dropped.
    pub async fn join_next(&mut self) -> Result<(InstanceId, Result<()>)> {
        if self.instances.is_empty() {
            bail!("No instances in InstanceManager");
        }
        let (instance_id, result) = self
            .completions_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("Instance completion channel closed"))?;
        self.instances.remove(&instance_id);
        Ok((instance_id, result))
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn instance_ids(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.instances.keys().copied()
    }

    pub fn send_view_event(&self, path: impl Into<ViewPath>, event: ViewEvent) -> Result<()> {
        let (instance, view) = path.into().into();
        self.send_event(instance, ApplicationMessage::View(view, event))
    }

    pub fn send_event(&self, instance_id: InstanceId, event: ApplicationMessage) -> Result<()> {
        let instance = self.get_instance(instance_id)?;

        instance
            .events_tx
            .send(event)
            .with_context(|| format!("Failed to send event to instance {:?}", instance_id))
    }

    fn get_instance(&self, instance: InstanceId) -> Result<&RunningInstance> {
        self.instances
            .get(&instance)
            .ok_or_else(|| anyhow!("Instance {:?} does not exist", instance))
    }
}

/// Run one instance future on its own dedicated OS thread (ADR 0009).
///
/// The thread is detached: it reports `(InstanceId, result)` on the completion channel and
/// the manager never joins or aborts it — shutdown is cooperative and the process end reaps
/// threads that outlive the shutdown deadline. The runtime is built inside the closure so
/// its lifetime encloses `block_on`: dropping a runtime before `spawn_blocking` work
/// completes would abort the pty reader.
fn spawn_instance_thread<F>(
    instance_id: InstanceId,
    runtime_kind: RuntimeKind,
    instance_future: F,
    instance_task_context: TaskContext,
    completions_tx: UnboundedSender<(InstanceId, Result<()>)>,
) -> std::io::Result<thread::JoinHandle<()>>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    thread::Builder::new()
        .name(format!("instance {instance_id:?}"))
        .spawn(move || {
            let runtime = match runtime_kind {
                RuntimeKind::CurrentThread => tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build(),
                RuntimeKind::MultiThread => tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build(),
            };
            let runtime = match runtime {
                Ok(runtime) => runtime,
                // Report the failure on the completion channel instead of panicking: a
                // panic below would skip the send and leave `join_next` waiting forever.
                Err(e) => {
                    let _ = completions_tx.send((instance_id, Err(e.into())));
                    return;
                }
            };
            let result = runtime.block_on(task_context::with_context(
                instance_task_context,
                AssertUnwindSafe(instance_future).catch_unwind(),
            ));
            let result = match result {
                Ok(r) => r,
                Err(e) => {
                    let message = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("<non-string panic payload>");
                    Err(anyhow!("Instance panicked : {message}"))
                }
            };
            let _ = completions_tx.send((instance_id, result));
        })
}
