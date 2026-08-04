use std::any::Any;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc::{UnboundedReceiver, WeakUnboundedSender};
use tokio::sync::oneshot;

use winit::dpi::PhysicalSize;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowAttributes;

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::{ApplicationEvent, ApplicationMessage, Frame, PresentationId, ViewId};
use massive_geometry::SizePx;
use massive_scene::ChangeCollector;
use massive_util::CoalescingReceiver;

use crate::shell::ShellCommand;
use crate::{Scene, ShellWindow};

/// The [`ApplicationContext`] is the application's connection to the outer world. It allows it to create
/// new windows and to wait for events while also forwarding scene changes to the renderer.
///
/// In addition to that it provides an animator that is updated with each event coming from the
/// shell.
#[derive(Debug)]
pub struct ApplicationContext {
    // We use this to send `ApplyAnimations` from the renderers.
    event_sender: WeakUnboundedSender<ApplicationMessage>,
    event_receiver: CoalescingReceiver<ApplicationMessage>,
    // Used for stuff that needs to run on the event loop thread. Like Window creation, for example.
    pub(crate) event_loop_proxy: EventLoopProxy<ShellCommand>,

    // Robustness: Should probably an event loop query. May be different for different windows and
    // or when a window is moved?
    monitor_scale_factor: f64,

    animation_coordinator: AnimationCoordinator,
    movement_runtime: MovementRuntime,
}

impl ApplicationContext {
    pub(crate) fn new(
        event_sender: WeakUnboundedSender<ApplicationMessage>,
        event_receiver: UnboundedReceiver<ApplicationMessage>,
        event_loop_proxy: EventLoopProxy<ShellCommand>,
        monitor_scale_factor: f64,
    ) -> Self {
        Self {
            event_sender,
            event_receiver: event_receiver.into(),
            event_loop_proxy,
            monitor_scale_factor,
            animation_coordinator: AnimationCoordinator::new(),
            movement_runtime: MovementRuntime::default(),
        }
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.monitor_scale_factor
    }

    /// Creates a new scene with a new change collector.
    pub fn new_scene(&self) -> Scene {
        Scene::new(Arc::new(ChangeCollector::default()))
    }

    /// Creates a new scene with a caller-provided change collector.
    pub fn new_scene_with_change_collector(&self, collector: Arc<ChangeCollector>) -> Scene {
        Scene::new(collector)
    }

    /// The application movement runtime for mounting long-lived movements.
    pub fn movement_runtime(&mut self) -> &mut MovementRuntime {
        &mut self.movement_runtime
    }

    /// Bundle a scene with the application's animation clock for one update cycle.
    pub fn frame<'scene, 'context>(
        &'context mut self,
        scene: &'scene Scene,
    ) -> Frame<'scene, 'context> {
        Frame::new(
            scene,
            &mut self.animation_coordinator,
            &mut self.movement_runtime,
        )
    }

    /// Creates a new window.
    ///
    /// Async because it needs to communicate with the application's main thread on which the window
    /// is actually created.
    pub async fn new_window(&self, inner_size: impl Into<SizePx>) -> Result<ShellWindow> {
        let (on_created, when_created) = oneshot::channel();
        let view_id = ViewId::new();
        let presentation_id = PresentationId::new();
        let inner_size = inner_size.into();
        let attributes = WindowAttributes::default()
            .with_inner_size(PhysicalSize::new(inner_size.width, inner_size.height));
        self.event_loop_proxy
            .send_event(ShellCommand::CreateWindow {
                view_id,
                attributes: attributes.into(),
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;

        let window = when_created.await??;
        Ok(ShellWindow::new(
            view_id,
            presentation_id,
            window,
            self.event_loop_proxy.clone(),
            self.event_sender.clone(),
        ))
    }

    /// Wait for multiple application events treating custom events of type `T`. If custom events
    /// are received that are not of type `T`, this results in an error.
    ///
    /// Right now, custom events may be produced by the [`MovementRuntime`].
    pub async fn wait_for_events<T: Any>(&mut self) -> Result<Vec<ApplicationEvent<T>>> {
        let events = self.event_receiver.recv_all().await?;

        let mut application_events = Vec::with_capacity(events.len());
        for event in events {
            match event {
                ApplicationMessage::View(view_id, view_event) => {
                    application_events.push(ApplicationEvent::View(view_id, view_event));
                }
                ApplicationMessage::ApplyAnimations(presentation_id) => {
                    self.animation_coordinator
                        .upgrade_to_apply_animations_cycle();
                    let completion_events = self
                        .movement_runtime
                        .apply_animations(self.animation_coordinator.animation_time());
                    application_events.push(ApplicationEvent::ApplyAnimations(presentation_id));
                    application_events.extend(
                        completion_events
                            .into_iter()
                            .map(|event| {
                                let event = event.downcast::<T>().map_err(|_| {
                                    anyhow!("movement completion event has the wrong type")
                                })?;
                                Ok(ApplicationEvent::Custom(*event))
                            })
                            .collect::<Result<Vec<_>>>()?,
                    );
                }
                ApplicationMessage::Shutdown(instance_id) => {
                    application_events.push(ApplicationEvent::Shutdown(instance_id));
                }
            }
        }

        Ok(application_events)
    }
}
