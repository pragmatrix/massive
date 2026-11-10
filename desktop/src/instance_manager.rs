use std::collections::HashMap;
use std::io;
use std::panic::AssertUnwindSafe;
use std::thread;

use anyhow::{Context, anyhow, bail};
use derive_more::{Debug, From, Into};
use futures::FutureExt;
use log::warn;
use panic_message::panic_message as panic_payload_message;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::task_context::{self, TaskContext};
use massive_applications::{
    Application, CreationMode, InstanceChange, InstanceContext, InstanceEnvironment,
    InstanceFuture, InstanceId, InstanceMessage, RuntimeKind, ViewEvent, ViewId,
};
use massive_scene::AnyCollector;
use massive_scene::{Location, Ref};
use massive_shell::Result;
use massive_util::ChangeSet;

/// Prefix for the OS thread name of each instance thread; the [`InstanceId`] follows
/// its debug form.
const INSTANCE_THREAD_NAME_PREFIX: &str = "massive ";

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
    /// Sender the instance threads report their end on. Held by the manager so the receiver in
    /// `completions_rx` stays open between spawns.
    completions_tx: UnboundedSender<InstanceEnd>,
    completions_rx: UnboundedReceiver<InstanceEnd>,
}

/// What an ended instance reports.
#[derive(Debug)]
pub struct InstanceEnd {
    pub instance: InstanceId,
    pub result: Result<()>,
    /// The changes the instance's task made after its final submission, such as the releases of
    /// what it held until the end. The desktop integrates them after the instance's last outputs,
    /// so those objects still end (ADR 0040).
    pub final_changes: ChangeSet<InstanceChange>,
}

#[derive(Debug)]
struct RunningInstance {
    #[allow(unused)]
    application_name: String,
    events_tx: UnboundedSender<InstanceMessage>,
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
        let instance_future = application.run(instance_context);
        let runtime_kind = application.runtime_kind();
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
                application_name: application.name().to_owned(),
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

    /// Begin the shutdown of an instance by sending [`InstanceMessage::Shutdown`]. Returns
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
            .send(InstanceMessage::Shutdown(instance_id))
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
    pub async fn join_next(&mut self) -> Result<InstanceEnd> {
        if self.instances.is_empty() {
            bail!("No instances in InstanceManager");
        }
        let end = self
            .completions_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("Instance completion channel closed"))?;
        self.instances.remove(&end.instance);
        Ok(end)
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
        self.send_event(instance, InstanceMessage::View(view, event))
    }

    pub fn send_event(&self, instance_id: InstanceId, event: InstanceMessage) -> Result<()> {
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
/// The thread is detached: it reports its [`InstanceEnd`] on the completion channel and
/// the manager never joins or aborts it — shutdown is cooperative and the process end reaps
/// threads that outlive the shutdown deadline.
fn spawn_instance_thread(
    instance_id: InstanceId,
    runtime_kind: RuntimeKind,
    instance_future: InstanceFuture,
    instance_task_context: TaskContext,
    completions_tx: UnboundedSender<InstanceEnd>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name(format!("{INSTANCE_THREAD_NAME_PREFIX}{instance_id:?}"))
        .spawn(move || {
            // The send is the only exit: every failure of [run_instance_future] becomes a
            // completion, so the manager never waits on an instance that cannot report.
            let (result, final_changes) =
                run_instance_future(runtime_kind, instance_future, instance_task_context);
            let _ = completions_tx.send(InstanceEnd {
                instance: instance_id,
                result,
                final_changes,
            });
        })
}

/// Build the instance runtime and drive the instance future to completion, converting a
/// panic into an error result. Also returns the changes the instance's task made after its final
/// submission.
fn run_instance_future(
    runtime_kind: RuntimeKind,
    instance_future: InstanceFuture,
    instance_task_context: TaskContext,
) -> (Result<()>, ChangeSet<InstanceChange>) {
    // The runtime is built inside the thread closure so its lifetime encloses `block_on`:
    // dropping a runtime before `spawn_blocking` work completes would abort the pty reader.
    let runtime = match runtime_kind {
        RuntimeKind::CurrentThread => Builder::new_current_thread(),
        RuntimeKind::MultiThread => Builder::new_multi_thread(),
    }
    .enable_all()
    .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            // The future holds the instance's view parent, whose release has to reach the
            // desktop (ADR 0040).
            let final_changes =
                task_context::drop_with_context(instance_task_context, instance_future);
            return (
                Err(error).context("building an instance runtime"),
                final_changes,
            );
        }
    };
    let (result, final_changes) = runtime.block_on(task_context::with_context_and_final_changes(
        instance_task_context,
        AssertUnwindSafe(instance_future).catch_unwind(),
    ));
    let result = match result {
        Ok(result) => result,
        // `catch_unwind` yields `Box<dyn Any + Send>`, exactly the payload type
        // `panic_message` takes — it unpacks `&str`/`String` payloads like rustc's
        // default handler and falls back to "Box<dyn Any>" for anything else.
        Err(payload) => Err(anyhow!(
            "Instance panicked : {}",
            panic_payload_message(&payload)
        )),
    };
    (result, final_changes)
}

#[cfg(test)]
mod tests {
    use massive_applications::prelude::Submit;
    use massive_applications::{InstanceChange, InstanceOutput, InstanceParameters};
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::prelude::*;
    use massive_scene::{LifetimeStage, RenderChange, RenderSceneChange, SceneChange};
    use uuid::Uuid;

    use super::*;

    /// An instance holds its view parent until it ends, so the parent's last reference is
    /// released after the instance's final submission. That release must still reach the desktop,
    /// or the view parent never ends (ADR 0040).
    #[tokio::test]
    async fn an_ended_instance_releases_its_view_parent() -> Result<()> {
        task_context::with_context(desktop_task_context(), async {
            let (outputs_tx, mut outputs_rx) = unbounded_channel();
            let mut manager = InstanceManager::new(InstanceEnvironment::new(outputs_tx, 1.0, None));
            let (transform, location) = identity_location().submit();
            let location_id = location.id();
            let application = Application::new("ends", |context| async move {
                drop(context);
                Ok(())
            });
            manager.spawn(
                Uuid::new_v4().into(),
                &application,
                CreationMode::New(InstanceParameters::default()),
                location.to_ref(),
            )?;
            drop((transform, location));
            let end = manager.join_next().await?;
            end.result?;

            // Integrate like the desktop: its own changes first, then each instance output, then
            // what processing the outputs released.
            let mut changes = task_context::take_changes::<SceneChange>().release();
            while let Ok((_, output)) = outputs_rx.try_recv() {
                let InstanceOutput::Submission(submission) = output else {
                    continue;
                };
                for change in submission.into_parts().0.release() {
                    if let InstanceChange::Scene(change) = change {
                        changes.push(change);
                    }
                }
            }
            changes.extend(task_context::take_changes::<SceneChange>().release());
            for change in end.final_changes.release() {
                if let InstanceChange::Scene(change) = change {
                    changes.push(change);
                }
            }

            let ended = LifetimeStage::default().resolve(changes).into_iter().any(|change| {
                matches!(change, RenderSceneChange::Location(RenderChange::Delete(id)) if id == location_id)
            });
            assert!(ended, "the ended instance's view parent stays alive");
            Ok(())
        })
        .await
    }

    fn desktop_task_context() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<SceneChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            FontManager::bare(ShapingEngineKind::available()[0]).new_shaping_context(),
        )
    }
}
