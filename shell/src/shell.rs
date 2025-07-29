use std::{future::Future, sync::Arc};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use anyhow::Result;
use log::info;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use wgpu::{Instance, Surface, SurfaceTarget};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{application_context::RenderPacing, ApplicationContext, ShellWindow};
use massive_animation::Tickery;

pub async fn run<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // Robustness: may use unbounded channels.
    let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    let tickery = Arc::new(Tickery::new(Instant::now()));

    let application_context = ApplicationContext {
        event_receiver,
        event_loop_proxy,
        tickery,
        render_pacing: RenderPacing::default(),
    };

    let (result_tx, mut result_rx) = oneshot::channel();
    let _application_task = tokio::spawn(async move {
        let r = application(application_context).await;
        // Found no way to retrieve the result via JoinHandle, so a return via a onceshot channel
        // must do.
        result_tx
            .send(Some(r))
            .expect("Internal Error: Failed to set the application result");
    });

    // Event loop

    {
        let mut winit_context = WinitApplicationHandler { event_sender };

        info!("Entering event loop");
        event_loop.run_app(&mut winit_context)?;
        info!("Exiting event loop");
    };

    // Check application's result
    if let Ok(Some(r)) = result_rx.try_recv() {
        info!("Application ended with {r:?}");
        r?;
    } else {
        // TODO: This should probably be an error, we want to the application to have full
        // control over the lifetime of the winit event loop.
        info!("Application did not end");
    }

    Ok(())
}

#[must_use]
#[derive(Debug, Copy, Clone)]
pub enum ControlFlow {
    Exit,
    Continue,
}

pub enum ShellRequest {
    CreateWindow {
        // Box because of large size.
        attributes: Box<WindowAttributes>,
        on_created: oneshot::Sender<Result<Window>>,
    },
    /// Surfaces need to be created on the main thread on macOS when a window handle is provided.
    CreateSurface {
        instance: Instance,
        target: SurfaceTarget<'static>,
        on_created: oneshot::Sender<Result<Surface<'static>>>,
    },
}

#[derive(Debug)]
pub enum ShellEvent {
    // Architecture: Seperate this into a separate WindowEvent, because ApplyAnimations isn't used
    // as a event pathway from the WinitApplicationHandler anymore.
    WindowEvent(WindowId, WindowEvent),
    ApplyAnimations,
}

impl ShellEvent {
    #[must_use]
    pub fn window_event_for(&self, window: &ShellWindow) -> Option<&WindowEvent> {
        match self {
            ShellEvent::WindowEvent(id, window_event) if *id == window.id() => Some(window_event),
            _ => None,
        }
    }

    #[must_use]
    pub fn window_event_for_id(&self, id: WindowId) -> Option<&WindowEvent> {
        match self {
            ShellEvent::WindowEvent(wid, window_event) if *wid == id => Some(window_event),
            _ => None,
        }
    }

    #[must_use]
    pub fn apply_animations(&self) -> bool {
        matches!(self, Self::ApplyAnimations)
    }
}

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}

struct WinitApplicationHandler {
    event_sender: UnboundedSender<ShellEvent>,
}

impl ApplicationHandler<ShellRequest> for WinitApplicationHandler {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Robustness: As recommended, wait for the resumed event before creating any window.
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellRequest) {
        match event {
            ShellRequest::CreateWindow {
                attributes,
                on_created,
            } => {
                let r = event_loop.create_window(*attributes);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
            ShellRequest::CreateSurface {
                instance,
                target,
                on_created,
            } => {
                let r = instance.create_surface(target);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event != WindowEvent::RedrawRequested {
            info!("{event:?}");
        }

        self.send_event(event_loop, ShellEvent::WindowEvent(window_id, event))
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, shell_event: ShellEvent) {
        if let Err(_e) = self.event_sender.send(shell_event) {
            // Don't log when we are already exiting.
            if !event_loop.exiting() {
                info!("Receiver for events dropped, exiting event loop: {_e:?}");
                event_loop.exit();
            }
        }
    }
}
