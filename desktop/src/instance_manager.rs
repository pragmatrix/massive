use std::{collections::HashMap, future::Future, panic::AssertUnwindSafe, pin::Pin};

use anyhow::anyhow;
use derive_more::{Debug, Deref};
use futures::FutureExt;
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::JoinSet,
};
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceContext, InstanceEvent, InstanceId, RenderPacing,
    ViewCreationInfo, ViewId,
};
use massive_renderer::FontManager;
use massive_shell::Result;

/// Manages running application instances with lifecycle control.
#[derive(Debug)]
pub struct InstanceManager {
    instances: HashMap<InstanceId, RunningInstance>,
    join_set: JoinSet<(InstanceId, Result<()>)>,
    requests_tx: UnboundedSender<(InstanceId, InstanceCommand)>,
}

#[derive(Debug)]
struct RunningInstance {
    #[allow(dead_code)]
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
    pub fn new(requests_tx: UnboundedSender<(InstanceId, InstanceCommand)>) -> Self {
        Self {
            instances: HashMap::new(),
            join_set: JoinSet::new(),
            requests_tx,
        }
    }

    /// Restore an instance (spawn it with CreationMode::Restore).
    /// This would typically be called after stopping an instance that needs to be restarted.
    #[allow(dead_code)]
    pub fn restore(&mut self, application: &Application, fonts: FontManager) -> Result<InstanceId> {
        // Note: Each spawn creates a new instance with a new ID.
        // Applications should handle state restoration via CreationMode::Restore.
        self.spawn(application, CreationMode::Restore, fonts)
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
        fonts: FontManager,
    ) -> Result<InstanceId> {
        let instance_id = InstanceId::from(Uuid::new_v4());
        let (events_tx, events_rx) = unbounded_channel();

        let instance_context = InstanceContext::new(
            instance_id,
            creation_mode,
            self.requests_tx.clone(),
            events_rx,
            fonts,
        );

        let instance_future = (application.run)(instance_context);
        self.join_set.spawn(async move {
            let result = AssertUnwindSafe(instance_future).catch_unwind().await;
            let result = match result {
                Ok(r) => r,
                Err(_) => Err(anyhow!("Instance panicked")),
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

    /// Wait for the next instance to complete and handle cleanup.
    /// Returns `Ok((instance_id, result))` when an instance completes, `Err` if the task was cancelled or the JoinSet is empty.
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
        let instance = self
            .instances
            .get_mut(&instance_id)
            .ok_or_else(|| anyhow!("Instance {:?} not found", instance_id))?;
        let view = instance
            .views
            .get_mut(&view_id)
            .ok_or_else(|| anyhow!("View {:?} not found", view_id))?;
        view.pacing = pacing;
        Ok(())
    }

    pub fn views(&self) -> impl Iterator<Item = (InstanceId, &ViewId, &ViewInfo)> {
        self.instances.iter().flat_map(|(instance_id, instance)| {
            instance
                .views
                .iter()
                .map(move |(view_id, info)| (*instance_id, view_id, info))
        })
    }

    #[allow(dead_code)]
    pub fn get_view(&self, instance_id: InstanceId, view_id: ViewId) -> Option<&ViewInfo> {
        self.instances
            .get(&instance_id)
            .and_then(|instance| instance.views.get(&view_id))
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

#[derive(Debug)]
pub struct Application {
    pub(crate) name: String,
    #[debug(skip)]
    pub(crate) run: RunInstanceBox,
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
