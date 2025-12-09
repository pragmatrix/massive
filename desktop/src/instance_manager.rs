use std::{collections::HashMap, panic::AssertUnwindSafe};

use anyhow::{Context, anyhow};
use derive_more::{Debug, Deref};
use futures::FutureExt;
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceContext, InstanceEnvironment, InstanceEvent, InstanceId, RenderPacing,
    ViewCreationInfo, ViewEvent, ViewId, ViewRole,
};
use massive_shell::Result;

use crate::application_registry::Application;

/// Manages running application instances with lifecycle control.
#[derive(Debug)]
pub struct InstanceManager {
    instances: HashMap<InstanceId, RunningInstance>,
    environment: InstanceEnvironment,
    join_set: JoinSet<(InstanceId, Result<()>)>,
}

#[derive(Debug)]
struct RunningInstance {
    application_name: String,
    #[allow(dead_code)]
    creation_mode: CreationMode,
    events_tx: UnboundedSender<InstanceEvent>,
    views: HashMap<ViewId, ViewInfo>,
}

#[derive(Debug, Deref)]
pub struct ViewInfo {
    #[deref]
    pub creation_info: ViewCreationInfo,
    pub pacing: RenderPacing,
}

impl InstanceManager {
    pub fn new(environment: InstanceEnvironment) -> Self {
        Self {
            environment,
            instances: HashMap::new(),
            join_set: JoinSet::new(),
        }
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
            instance_id,
            creation_mode,
            self.environment.clone(),
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
                creation_mode,
                events_tx,
                views: HashMap::new(),
            },
        );

        Ok(instance_id)
    }

    /// Wait for the next instance to complete and handle cleanup.
    ///
    /// Returns `Ok((instance_id, result))` when an instance completes, `Err` if the task was
    /// cancelled or the JoinSet is empty.
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

    pub fn send_view_event(
        &self,
        instance_id: InstanceId,
        view_id: ViewId,
        event: ViewEvent,
    ) -> Result<()> {
        self.send_event(instance_id, InstanceEvent::View(view_id, event))
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

    pub fn add_view(&mut self, instance_id: InstanceId, creation_info: ViewCreationInfo) {
        if let Some(instance) = self.instances.get_mut(&instance_id) {
            let id = creation_info.id;
            let info = ViewInfo {
                creation_info,
                pacing: RenderPacing::default(),
            };
            instance.views.insert(id, info);
        }
    }

    pub fn remove_view(&mut self, instance_id: InstanceId, id: ViewId) {
        if let Some(instance) = self.instances.get_mut(&instance_id) {
            instance.views.remove(&id);
        }
    }

    pub fn update_view_pacing(
        &mut self,
        instance_id: InstanceId,
        view_id: ViewId,
        pacing: RenderPacing,
    ) -> Result<()> {
        let instance = self.mut_instance(instance_id)?;
        let view = instance
            .views
            .get_mut(&view_id)
            .ok_or_else(|| anyhow!("View {:?} not found", view_id))?;
        view.pacing = pacing;
        Ok(())
    }

    pub fn views(&self) -> impl Iterator<Item = (InstanceId, ViewId, &ViewInfo)> {
        self.instances.iter().flat_map(|(instance_id, instance)| {
            instance
                .views
                .iter()
                .map(|(view_id, info)| (*instance_id, *view_id, info))
        })
    }

    /// Returns the ViewInfo of a view if it's instance and the view exists.
    pub fn get_view(&self, instance_id: InstanceId, view_id: ViewId) -> Result<&ViewInfo> {
        self.get_instance(instance_id).and_then(|instance| {
            instance
                .views
                .get(&view_id)
                .ok_or_else(|| anyhow!("View not found"))
        })
    }

    /// Returns the first view with the given role. Returns `None` if no view with that role is
    /// found and an error if the instance does not exist.
    pub fn get_view_by_role(
        &self,
        instance_id: InstanceId,
        role: ViewRole,
    ) -> Result<Option<ViewId>> {
        Ok(self
            .get_instance(instance_id)?
            .views
            .iter()
            .find(|(_, info)| info.role == role)
            .map(|(id, _)| *id))
    }

    pub fn get_application_name(&self, instance: InstanceId) -> Result<&str> {
        self.get_instance(instance)
            .map(|ri| ri.application_name.as_str())
    }

    fn get_instance(&self, instance: InstanceId) -> Result<&RunningInstance> {
        self.instances
            .get(&instance)
            .ok_or_else(|| anyhow!("Internal error: Instance {:?} does not exist", instance))
    }

    fn mut_instance(&mut self, instance: InstanceId) -> Result<&mut RunningInstance> {
        self.instances
            .get_mut(&instance)
            .ok_or_else(|| anyhow!("Internal error: Instance {:?} does not exist", instance))
    }

    /// Returns the effective pacing across all views.
    /// If at least one view has Smooth pacing, returns Smooth; otherwise returns Fast.
    pub fn effective_pacing(&self) -> RenderPacing {
        if self
            .instances
            .values()
            .flat_map(|i| i.views.values())
            .any(|info| info.pacing == RenderPacing::Smooth)
        {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        }
    }
}
