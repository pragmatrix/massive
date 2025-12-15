use anyhow::{Result, anyhow};
use massive_geometry::SizePx;
use tokio::sync::{
    mpsc::{UnboundedReceiver, WeakUnboundedSender},
    oneshot,
};
use winit::{dpi::PhysicalSize, event_loop::EventLoopProxy, window::WindowAttributes};

use crate::{Scene, ShellEvent, ShellWindow, shell::ShellCommand};

use massive_animation::AnimationCoordinator;
use massive_util::CoalescingReceiver;

/// The [`ApplicationContext`] is the application's connection to the outer world. It allows it to create
/// new windows and to wait for events while also forwarding scene changes to the renderer.
///
/// In addition to that it provides an animator that is updated with each event coming from the
/// shell.
#[derive(Debug)]
pub struct ApplicationContext {
    // We use this to send ApplyAnimations from the renderers.
    event_sender: WeakUnboundedSender<ShellEvent>,
    event_receiver: CoalescingReceiver<ShellEvent>,
    // Used for stuff that needs to run on the event loop thread. Like Window creation, for example.
    pub(crate) event_loop_proxy: EventLoopProxy<ShellCommand>,

    // Robustness: Should probably an event loop query. May be different for different windows and
    // or when a window is moved?
    monitor_scale_factor: f64,

    animation_coordinator: AnimationCoordinator,
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
        }
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
        self.monitor_scale_factor
    }

    pub fn animation_coordinator(&self) -> &AnimationCoordinator {
        &self.animation_coordinator
    }

    /// Creates a new scene with the shared animation coordinator.
    pub fn new_scene(&self) -> Scene {
        Scene::new(self.animation_coordinator.clone())
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
    /// This function is cancel safe _and_ must be used in a atomic fashion (i.e. not preserved in a
    /// select! loop with &mut reference to the returning future).
    ///
    /// `renderer` is needed here so that we know when the renderer finished in animation mode and a
    /// [`ShellEvent::ApplyAnimations`] can be produced.
    pub async fn wait_for_shell_event(&mut self) -> Result<ShellEvent> {
        let event = self.event_receiver.recv().await?;

        if matches!(event, ShellEvent::ApplyAnimations(..)) {
            self.animation_coordinator.upgrade_to_apply_animations();
        }

        Ok(event)
    }
}
