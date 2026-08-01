use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use anyhow::{anyhow, bail};
use log::{debug, error, info, warn};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use wgpu::{Surface, SurfaceTarget};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopClosed, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use massive_applications::{ApplicationMessage, ViewEvent, ViewId};

use crate::ApplicationContext;
use crate::shell_window::ShellWindowShared;

const FALLBACK_SCALE_FACTOR: f64 = 1.;

/// Starts the shell.
///
/// This runs `application` with `tokio::spawn` on the tokio threadpool and waits for its
/// completion. It also executes the winit event loop and blocks until it returns. This gives
/// clients the option to run the event loop on the main thread, which some platforms require.
pub fn run<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    // _Try_ to instantiate env logger (main may already initialized it).
    let _ = env_logger::try_init();

    #[cfg(feature = "metrics")]
    if let Ok(push_gateway) = std::env::var("MASSIVE_METRICS_PUSHGATEWAY") {
        use std::time::Duration;

        match metrics_exporter_prometheus::PrometheusBuilder::new().with_push_gateway(
            push_gateway,
            Duration::from_secs(1),
            None,
            None,
            false,
        ) {
            Ok(builder) => {
                if let Err(e) = builder.install() {
                    log::warn!("Failed to install Prometheus metrics exporter: {}", e);
                }
            }
            Err(e) => {
                log::warn!("Failed to create Prometheus metrics builder: {}", e);
            }
        }
    } else {
        log::info!("Metrics disabled: MASSIVE_METRICS_PUSHGATEWAY not set");
    }

    // Power up a tokio runtime, if none is running yet.

    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            // Already inside a Tokio runtime.
            run_with_tokio(application)
        }
        Err(_) => {
            // Create and enter a multi-thread runtime so tokio::spawn can run while the event loop blocks.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let _guard = runtime.enter();
            let r = run_with_tokio(application);
            drop(_guard);
            r
        }
    }
}

fn run_with_tokio<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    let spawn_application = |application_context: ApplicationContext| {
        let _application_task = tokio::spawn(async move {
            let event_loop_proxy = application_context.event_loop_proxy.clone();
            let r = application(application_context).await;
            if let Err(EventLoopClosed(ShellCommand::ApplicationEnded(r))) =
                event_loop_proxy.send_event(ShellCommand::ApplicationEnded(r))
            {
                error!("Application ended after the event loop exited: {r:?}");
            }
        });
    };

    // Event loop

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

#[derive(Debug)]
pub(crate) enum ShellCommand {
    CreateWindow {
        view_id: ViewId,
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
        proxy: EventLoopProxy<ShellCommand>,
        // ADR: Option because we need to move it out.
        // Robustness: use a replace_with variant, so that we don't need an `Option<Box<..>>` here.
        spawner: Option<ApplicationSpawner>,
    },
    Running {
        event_sender: UnboundedSender<ApplicationMessage>,
        views: HashMap<WindowId, ViewId>,
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

impl ApplicationHandler<ShellCommand> for WinitApplicationHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initializing { proxy, spawner } = self else {
            panic!("Resumed called in an invalid state");
        };

        crate::platform::initialize_platform_menu();

        let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

        let scale_factor = event_loop
            .primary_monitor()
            .map(|pm| pm.scale_factor())
            .unwrap_or_else(|| {
                warn!("Failed to query the current monitor's scale factor, setting to {FALLBACK_SCALE_FACTOR}");
                FALLBACK_SCALE_FACTOR
            });

        let application_context = ApplicationContext::new(
            event_sender.downgrade(),
            event_receiver,
            proxy.clone(),
            scale_factor,
        );

        (spawner.take().unwrap())(application_context);
        *self = Self::Running {
            event_sender,
            views: HashMap::new(),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellCommand) {
        match event {
            ShellCommand::CreateWindow {
                view_id,
                attributes,
                on_created,
            } => {
                let Self::Running { views, .. } = self else {
                    panic!(
                        "Received CreateWindow user event while WinitApplicationHandler is not Running"
                    );
                };
                let r = event_loop.create_window(*attributes);
                if let Ok(window) = &r {
                    views.insert(window.id(), view_id);
                }
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
            ShellCommand::DestroyWindow { window } => {
                let Self::Running { views, .. } = self else {
                    panic!(
                        "Received DestroyWindow user event while WinitApplicationHandler is not Running"
                    );
                };
                views.remove(&window.id());
                info!("Destroying window");
                drop(window);
            }
            ShellCommand::CreateSurface {
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
            ShellCommand::ApplicationEnded(r) => {
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
            debug!("Received: {event:?}");
        }

        // Don't send Window destroyed events for now, the Window is already gone, no need to handle
        // this. This might also happen when the system is winding down (i.e. we are already not
        // anymore in the Running state)
        let view_event = match self {
            Self::Running { views, .. } if event != WindowEvent::Destroyed => views
                .get(&window_id)
                .copied()
                .zip(ViewEvent::from_window_event(&event)),
            _ => None,
        };

        if let Some((view_id, event)) = view_event {
            self.send_event(event_loop, ApplicationMessage::View(view_id, event))
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        replace_with::replace_with_or_abort(self, |state| {
            let final_result: Result<()> = if let Self::Ended { application_result } = state {
                // Detail: Don't output the error here. We'll do this later anyway.
                //
                // Robustness: We have to output the error here because the application may hang?.
                if let Err(e) = &application_result {
                    error!("Application ended: {e:?}");
                } else {
                    info!("Application ended");
                }
                application_result
            } else {
                Err(anyhow!("Event loop exited, but application did not end"))
            };

            Self::Exited { final_result }
        });
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, event: ApplicationMessage) {
        let Self::Running { event_sender, .. } = self else {
            error!("Cannot send shell event: application handler must be in the running state.");
            return;
        };

        if let Err(e) = event_sender.send(event) {
            // Don't log when we are already exiting.
            if !event_loop.exiting() {
                info!("Receiver for events dropped, exiting event loop: {e:?}");
                event_loop.exit();
            }
        }
    }
}
