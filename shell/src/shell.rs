use std::{future::Future, mem, sync::Arc};

use anyhow::{Result, anyhow, bail};
use log::{error, info};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use wgpu::{Surface, SurfaceTarget};
use winit::{
    application::ApplicationHandler,
    event::{DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopClosed, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{ApplicationContext, ShellWindow, shell_window::ShellWindowShared};

/// Starts the shell.
///
/// This runs `application` with `tokio::spawn` on the tokio threadpool and waits for its
/// completion. It also executes the winit event loop and blocks until it returns. This gives
/// clients the option to run the event loop on the main thread, which some platforms require.
///
/// This function is not async, but the tokio runtime _must_ be created and this function's async
/// caller must be called using the runtime's block_on() function (which #[tokio::main] does).
pub fn run<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    // _Try_ to instantiate env logger (main may already initialized it).
    let _ = env_logger::try_init();

    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    let spawn_application = |application_context: ApplicationContext| {
        let _application_task = tokio::spawn(async move {
            let event_loop_proxy = application_context.event_loop_proxy.clone();
            let r = application(application_context).await;
            if let Err(EventLoopClosed(ShellRequest::ApplicationEnded(r))) =
                event_loop_proxy.send_event(ShellRequest::ApplicationEnded(r))
            {
                error!("Application ended after the event loop exited: {r:?}");
            }
        });
    };

    // Event loop

    {
        let mut winit_context = WinitApplicationHandler::Initializing {
            proxy: event_loop_proxy,
            spawner: Some(Box::new(spawn_application)),
        };

        info!("Entering event loop");
        event_loop.run_app(&mut winit_context)?;
        info!("Exited event loop");

        let WinitApplicationHandler::Exited { final_result } = winit_context else {
            bail!("Internal error: Exited event loop, but it was never actually exiting");
        };

        final_result
    }
}

// Robustness: Try to remove the Clone requirement.
#[derive(Debug, Clone)]
pub enum ShellEvent {
    // Architecture: Separate this into a separate WindowEvent, because ApplyAnimations isn't used
    // as a event pathway from the WinitApplicationHandler anymore.
    WindowEvent(WindowId, WindowEvent),
    ApplyAnimations,
}

#[derive(Debug)]
pub(crate) enum ShellRequest {
    CreateWindow {
        // Box because of large size.
        attributes: Box<WindowAttributes>,
        on_created: oneshot::Sender<Result<Window>>,
    },
    DestroyWindow {
        window: Window,
    },
    /// Surfaces need to be created on the main thread on macOS when a window handle is provided.
    CreateSurface {
        instance: wgpu::Instance,
        window: Arc<ShellWindowShared>,
        on_created: oneshot::Sender<Result<Surface<'static>>>,
    },
    ApplicationEnded(Result<()>),
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

    pub(crate) fn skip_key(&self) -> Option<ShellEventSkipKey> {
        match self {
            ShellEvent::WindowEvent(window_id, window_event) => match window_event {
                WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::RedrawRequested => {
                    Some(ShellEventSkipKey::WindowEvent(
                        *window_id,
                        None,
                        mem::discriminant(window_event),
                    ))
                }
                WindowEvent::CursorMoved { device_id, .. } => Some(ShellEventSkipKey::WindowEvent(
                    *window_id,
                    Some(*device_id),
                    mem::discriminant(window_event),
                )),
                _ => None,
            },
            ShellEvent::ApplyAnimations => Some(ShellEventSkipKey::ApplyAnimations),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShellEventSkipKey {
    ApplyAnimations,
    WindowEvent(WindowId, Option<DeviceId>, mem::Discriminant<WindowEvent>),
}

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}

/// ADR: We move the application into the event loop handler.
/// - Because we need to scale_factor() to be passed _to_ application. This does not work on Wayland.
enum WinitApplicationHandler {
    Initializing {
        proxy: EventLoopProxy<ShellRequest>,
        // ADR: Option because we need to move it out.
        // Robustness: use a replace_with variant, so that we don't need an Option<Box<..>> here.
        spawner: Option<ApplicationSpawner>,
    },
    Running {
        event_sender: UnboundedSender<ShellEvent>,
    },
    Ended {
        application_result: Result<()>,
    },
    Exited {
        final_result: Result<()>,
    },
}

/// Type alias for the application spawner closure.
type ApplicationSpawner = Box<dyn FnOnce(ApplicationContext)>;

impl ApplicationHandler<ShellRequest> for WinitApplicationHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initializing { proxy, spawner } = self else {
            panic!("Resumed called in an invalid state");
        };

        let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

        let scale_factor = event_loop.primary_monitor().map(|pm| pm.scale_factor());

        let application_context =
            ApplicationContext::new(event_receiver, proxy.clone(), scale_factor);

        (spawner.take().unwrap())(application_context);
        *self = Self::Running { event_sender }
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
            ShellRequest::DestroyWindow { window } => {
                drop(window);
            }
            ShellRequest::CreateSurface {
                instance,
                window,
                on_created,
            } => {
                let target: SurfaceTarget<'static> = window.into();
                let r = instance.create_surface(target);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
            ShellRequest::ApplicationEnded(r) => {
                *self = Self::Ended {
                    application_result: r,
                };
                event_loop.exit();
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

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        replace_with::replace_with_or_abort(self, |state| {
            let final_result: Result<()> = if let Self::Ended { application_result } = state {
                // Detail: Don't output the error here. We'll do this later anyway.
                info!("Application ended");
                application_result
            } else {
                Err(anyhow!("Event loop exited, but application did not end"))
            };

            Self::Exited { final_result }
        });
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, shell_event: ShellEvent) {
        let Self::Running { event_sender, .. } = self else {
            error!("Cannot send event: application handler not in running state.");
            return;
        };

        if let Err(e) = event_sender.send(shell_event) {
            // Don't log when we are already exiting.
            if !event_loop.exiting() {
                info!("Receiver for events dropped, exiting event loop: {e:?}");
                event_loop.exit();
            }
        }
    }
}
