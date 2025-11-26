use std::collections::VecDeque;

use anyhow::{Result, anyhow, bail};
use tokio::sync::{
    mpsc::{UnboundedReceiver, WeakUnboundedSender, error::TryRecvError},
    oneshot,
};
use winit::{dpi, event_loop::EventLoopProxy, window::WindowAttributes};

use crate::{Scene, ShellEvent, ShellWindow, message_filter, shell::ShellCommand};
use massive_animation::AnimationCoordinator;

/// The [`ApplicationContext`] is the application's connection to the outer world. It allows it to create
/// new windows and to wait for events while also forwarding scene changes to the renderer.
///
/// In addition to that it provides an animator that is updated with each event coming from the
/// shell.
#[derive(Debug)]
pub struct ApplicationContext {
    // We use this to send ApplyAnimations from the renderers.
    event_sender: WeakUnboundedSender<ShellEvent>,
    event_receiver: UnboundedReceiver<ShellEvent>,
    // Used for stuff that needs to run on the event loop thread. Like Window creation, for example.
    pub(crate) event_loop_proxy: EventLoopProxy<ShellCommand>,

    // Robustness: Should probably an event loop query. May be different for different windows and
    // or when a window is moved?
    monitor_scale_factor: f64,

    animation_coordinator: AnimationCoordinator,

    /// Pending events received but not yet delivered.
    pending_events: VecDeque<ShellEvent>,
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
            event_receiver,
            event_loop_proxy,
            monitor_scale_factor,
            animation_coordinator: AnimationCoordinator::new(),
            pending_events: Default::default(),
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
    pub async fn new_window(
        &self,
        // Ergonomics: Use a massive-geometry size type here.
        inner_size: impl Into<dpi::Size>,
    ) -> Result<ShellWindow> {
        let (on_created, when_created) = oneshot::channel();
        let attributes = WindowAttributes::default().with_inner_size(inner_size);
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
        loop {
            // Pull in every event we can get.
            loop {
                match self.event_receiver.try_recv() {
                    Ok(event) => self.pending_events.push_back(event),
                    Err(TryRecvError::Disconnected) => {
                        bail!("Internal Error: Shell shut down, no more events");
                    }
                    Err(TryRecvError::Empty) => {
                        break;
                    }
                }
            }

            // Skip Window events by key. This is to remove the lagging of resizes, redraws and
            // other events that are considered safe to skip without causing side effects.
            //
            // Robustness: Going from VecDequeue to Vec and back is a mess.
            {
                let events: Vec<ShellEvent> = message_filter::keep_last_per_key(
                    self.pending_events.iter().cloned().collect(),
                    |ev| ev.skip_key(),
                );

                self.pending_events = events.into_iter().collect();
            }

            if let Some(pending) = self.pending_events.pop_front() {
                // If this is an apply animations event, we have to upgrade the cycle we are in so
                // that the renderer know when this is finished, it can switch back to fast
                // rendering.
                if matches!(pending, ShellEvent::ApplyAnimations(..)) {
                    self.animation_coordinator.upgrade_to_apply_animations();
                }
                return Ok(pending);
            }

            // No events yet, wait.
            if let Some(event) = self.event_receiver.recv().await {
                self.pending_events.push_back(event);
            } else {
                bail!("Internal Error: Shell shut down, no more events")
            }
        }
    }
}
