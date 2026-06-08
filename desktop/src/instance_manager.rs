use std::collections::HashMap;
use std::panic::AssertUnwindSafe;

use anyhow::{Context, anyhow};
use derive_more::{Debug, From, Into};
use futures::FutureExt;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinSet;
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceContext, InstanceEnvironment, InstanceEvent, InstanceId, ViewEvent,
    ViewId,
};
use massive_scene::{Handle, Location, Object, ToLocation, Transform};
use massive_shell::{Result, Scene};

use crate::application_registry::Application;

/// Manages running application instances with lifecycle control.
#[derive(Debug)]
pub struct InstanceManager {
    instances: HashMap<InstanceId, RunningInstance>,
    environment: InstanceEnvironment,
    join_set: JoinSet<(InstanceId, Result<()>)>,
}

#[derive(Debug, Clone)]
pub struct InstanceRoot {
    transform: Handle<Transform>,
    location: Handle<Location>,
}

impl InstanceRoot {
    pub fn new(scene: &Scene) -> Self {
        let transform = Transform::IDENTITY.enter(scene);
        let location = transform.to_location().enter(scene);

        Self {
            transform,
            location,
        }
    }

    pub fn into_parts(self) -> (Handle<Transform>, Handle<Location>) {
        (self.transform, self.location)
    }
}

#[derive(Debug)]
struct RunningInstance {
    #[allow(unused)]
    application_name: String,
    events_tx: UnboundedSender<InstanceEvent>,
    root: InstanceRoot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, From, Into)]
pub struct ViewPath {
    pub instance: InstanceId,
    pub view: ViewId,
}

impl InstanceManager {
    pub fn new(environment: InstanceEnvironment) -> Self {
        Self {
            environment,
            instances: HashMap::new(),
            join_set: JoinSet::new(),
        }
    }

    /// Spawn a new instance of an application.
    pub fn spawn(
        &mut self,
        application: &Application,
        creation_mode: CreationMode,
        root: InstanceRoot,
    ) -> Result<InstanceId> {
        let instance_id = InstanceId::from(Uuid::new_v4());
        let (events_tx, events_rx) = unbounded_channel();

        let instance_context = InstanceContext::new(
            instance_id,
            creation_mode,
            self.environment.clone(),
            root.location.to_ref(),
            events_rx,
        );

        let instance_future = (application.run)(instance_context);
        self.join_set.spawn(async move {
            let result = AssertUnwindSafe(instance_future).catch_unwind().await;
            let result = match result {
                Ok(r) => r,
                Err(e) => Err(anyhow!("Instance panicked : {e:?}")),
            };
            (instance_id, result)
        });

        self.instances.insert(
            instance_id,
            RunningInstance {
                application_name: application.name.clone(),
                events_tx,
                root,
            },
        );

        Ok(instance_id)
    }

    /// Begin the shutdown of an instance by sending [`InstanceEvent::Shutdown`]. Returns immediately
    /// after sending the event
    pub fn request_shutdown(&self, instance_id: InstanceId) -> Result<()> {
        let instance = self.instances.get(&instance_id).ok_or_else(|| {
            anyhow!(
                "Failed to request a shutdown: Instance {:?} not found",
                instance_id
            )
        })?;

        instance
            .events_tx
            .send(InstanceEvent::Shutdown)
            .map_err(|_| {
                anyhow!(
                    "Failed to send shutdown event to instance {:?}",
                    instance_id
                )
            })
    }

    /// Wait for the next instance to complete and handle cleanup.
    ///
    /// Returns `Ok((instance_id, result))` when an instance completes, `Err` if the task was
    /// canceled or the JoinSet is empty.
    pub async fn join_next(&mut self) -> Result<(InstanceId, Result<()>)> {
        let join_result = self.join_set.join_next().await;
        let (instance_id, result) = join_result
            .ok_or_else(|| anyhow!("No instances in JoinSet"))?
            .map_err(|e| anyhow!("Task cancelled: {}", e))?;
        self.instances.remove(&instance_id);
        Ok((instance_id, result))
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn send_view_event(&self, path: impl Into<ViewPath>, event: ViewEvent) -> Result<()> {
        let (instance, view) = path.into().into();
        self.send_event(instance, InstanceEvent::View(view, event))
    }

    pub fn send_event(&self, instance_id: InstanceId, event: InstanceEvent) -> Result<()> {
        let instance = self.get_instance(instance_id)?;

        instance
            .events_tx
            .send(event)
            .with_context(|| format!("Failed to send event to instance {:?}", instance_id))
    }

    pub fn broadcast_event(&self, event: InstanceEvent) {
        for instance in self.instances.values() {
            let _ = instance.events_tx.send(event.clone());
        }
    }

    pub fn instance_root(&self, instance: InstanceId) -> Result<InstanceRoot> {
        self.get_instance(instance).map(|ri| ri.root.clone())
    }

    fn get_instance(&self, instance: InstanceId) -> Result<&RunningInstance> {
        self.instances
            .get(&instance)
            .ok_or_else(|| anyhow!("Instance {:?} does not exist", instance))
    }
}
