use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc::{UnboundedReceiver, WeakUnboundedSender};
use tokio::sync::oneshot;

use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::EventLoopProxy;
use winit::window::{WindowAttributes, WindowId};

use massive_animation::{AnimationContext, AnimationCoordinator, MovementRuntime};
use massive_applications::Frame;
use massive_geometry::SizePx;
use massive_scene::ChangeCollector;
use massive_util::CoalescingReceiver;

use crate::shell::ShellCommand;
use crate::{Scene, ShellEvent, ShellWindow};

#[derive(Debug)]
pub enum ApplicationEvent<T> {
    Window(WindowId, WindowEvent),
    Custom(T),
    ApplyAnimations(WindowId),
}

/// The [`ApplicationContext`] is the application's connection to the outer world. It allows it to create
/// new windows and to wait for events while also forwarding scene changes to the renderer.
///
/// In addition to that it provides an animator that is updated with each event coming from the
/// shell.
#[derive(Debug)]
pub struct ApplicationContext {
    // We use this to send `ApplyAnimations` from the renderers.
    event_sender: WeakUnboundedSender<ShellEvent>,
    event_receiver: CoalescingReceiver<ShellEvent>,
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
        event_sender: WeakUnboundedSender<ShellEvent>,
        event_receiver: UnboundedReceiver<ShellEvent>,
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

    // Temporary, until the `MovementRuntime` takes over.
    pub fn animation_context_mut(&mut self) -> &mut dyn AnimationContext {
        &mut self.animation_coordinator
    }

    /// Creates a new scene with a new change collector.
    pub fn new_scene(&self) -> Scene {
        Scene::new(Arc::new(ChangeCollector::default()))
    }

    /// Creates a new scene with a caller-provided change collector.
    pub fn new_scene_with_change_collector(&self, collector: Arc<ChangeCollector>) -> Scene {
        Scene::new(collector)
    }

    /// Bundle a scene with the application's animation clock for one update cycle.
    pub fn frame<'a>(&'a mut self, scene: &'a Scene) -> Frame<'a> {
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
        let inner_size = inner_size.into();
        let attributes = WindowAttributes::default()
            .with_inner_size(PhysicalSize::new(inner_size.width, inner_size.height));
        self.event_loop_proxy
            .send_event(ShellCommand::CreateWindow {
                attributes: attributes.into(),
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;

        let window = when_created.await??;
        Ok(ShellWindow::new(
            window,
            self.event_loop_proxy.clone(),
            self.event_sender.clone(),
        ))
    }

    /// Wait for the next shell event.
    ///
    /// This function is cancel safe _and_ must be used in an atomic fashion (i.e. not preserved in a
    /// `select!` loop with `&mut` reference to the returning future).
    ///
    /// `renderer` is needed here so that we know when the renderer finished in animation mode and a
    /// [`ShellEvent::ApplyAnimations`] can be produced.
    pub async fn wait_for_shell_event(&mut self) -> Result<ShellEvent> {
        let event = self.event_receiver.recv().await?;

        if matches!(event, ShellEvent::ApplyAnimations(..)) {
            self.animation_coordinator
                .upgrade_to_apply_animations_cycle();
        }

        Ok(event)
    }

    /// Wait for an application event treating custom events of type `T`. If custom events are
    /// received that are not of type `T`, this results in an error.
    ///
    /// Right now, custom events may be produced by the [`MovementRuntime`].
    pub async fn wait_for_event<T>(&mut self) -> Result<ApplicationEvent<T>> {
        let event = self.event_receiver.recv().await?;

        if matches!(event, ShellEvent::ApplyAnimations(..)) {
            self.animation_coordinator
                .upgrade_to_apply_animations_cycle();

            self.movement_runtime
                .apply_animations(&mut self.animation_coordinator);
        }

        let application_event = match event {
            ShellEvent::WindowEvent(window_id, window_event) => {
                ApplicationEvent::Window(window_id, window_event)
            }
            ShellEvent::ApplyAnimations(window_id) => ApplicationEvent::ApplyAnimations(window_id),
        };

        Ok(application_event)
    }
}
