use std::collections::HashMap;

use anyhow::anyhow;
use massive_animation::AnimationCoordinator;
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceContext, InstanceEvent, InstanceId,
};
use massive_shell::Result;

use crate::Application;

/// Manages running application instances with lifecycle control.
#[derive(Debug)]
pub struct InstanceManager {
    animation_coordinator: AnimationCoordinator,
    instances: HashMap<InstanceId, RunningInstance>,
    pub(crate) join_set: JoinSet<(InstanceId, Result<()>)>,
    requests_tx: UnboundedSender<(InstanceId, InstanceCommand)>,
}

#[derive(Debug)]
struct RunningInstance {
    #[allow(dead_code)]
    application_name: String,
    #[allow(dead_code)]
    creation_mode: CreationMode,
    events_tx: UnboundedSender<InstanceEvent>,
}

impl InstanceManager {
    pub fn new(
        animation_coordinator: AnimationCoordinator,
        requests_tx: UnboundedSender<(InstanceId, InstanceCommand)>,
    ) -> Self {
        Self {
            animation_coordinator,
            instances: HashMap::new(),
            join_set: JoinSet::new(),
            requests_tx,
        }
    }

    /// Restore an instance (spawn it with CreationMode::Restore).
    /// This would typically be called after stopping an instance that needs to be restarted.
    #[allow(dead_code)]
    pub fn restore(&mut self, application: &Application) -> Result<InstanceId> {
        // Note: Each spawn creates a new instance with a new ID.
        // Applications should handle state restoration via CreationMode::Restore.
        self.spawn(application, CreationMode::Restore)
    }

    /// Stop an instance gracefully by sending an Exit event.
    /// Returns immediately after sending the event; use wait_for_instance to wait for completion.
    #[allow(dead_code)]
    pub fn stop(&mut self, instance_id: InstanceId) -> Result<()> {
        let instance = self
            .instances
            .get(&instance_id)
            .ok_or_else(|| anyhow!("Instance {:?} not found", instance_id))?;

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

    /// Spawn a new instance of an application.
    pub fn spawn(
        &mut self,
        application: &Application,
        creation_mode: CreationMode,
    ) -> Result<InstanceId> {
        let instance_id = InstanceId::from(Uuid::new_v4());
        let (events_tx, events_rx) = unbounded_channel();

        let instance_context = InstanceContext::new(
            self.animation_coordinator.clone(),
            instance_id,
            creation_mode,
            self.requests_tx.clone(),
            events_rx,
        );

        let instance_future = (application.run)(instance_context);
        self.join_set.spawn(async move {
            let result = instance_future.await;
            (instance_id, result)
        });

        self.instances.insert(
            instance_id,
            RunningInstance {
                application_name: application.name.clone(),
                creation_mode,
                events_tx,
            },
        );

        Ok(instance_id)
    }

    /// Wait for a specific instance to complete.
    #[allow(dead_code)]
    pub async fn wait_for_instance(&mut self, target_id: InstanceId) -> Result<()> {
        while let Some(join_result) = self.join_set.join_next().await {
            let (instance_id, result) = join_result
                .unwrap_or_else(|e| (target_id, Err(anyhow!("Instance stopped: {}", e))));

            self.instances.remove(&instance_id);

            if instance_id == target_id {
                return result;
            }
        }
        Err(anyhow!("Instance {:?} not found in join set", target_id))
    }

    pub fn remove_instance(&mut self, instance_id: InstanceId) {
        self.instances.remove(&instance_id);
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn send_event(&self, instance_id: InstanceId, event: InstanceEvent) -> Result<()> {
        let instance = self
            .instances
            .get(&instance_id)
            .ok_or_else(|| anyhow!("Instance {:?} not found", instance_id))?;

        instance
            .events_tx
            .send(event)
            .map_err(|_| anyhow!("Failed to send event to instance {:?}", instance_id))
    }
}
