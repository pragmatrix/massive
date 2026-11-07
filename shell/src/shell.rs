use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use anyhow::Result;
use anyhow::{anyhow, bail};
use log::{debug, error, info, warn};
use tokio::runtime::RuntimeFlavor;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use wgpu::{Surface, SurfaceTarget};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopClosed, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use massive_applications::{Application, InstanceMessage, ViewEvent, ViewId, ime_kind};
use massive_renderer::{FontManager, FontPolicy};

use crate::instance_host::{self, Startup};
use crate::platform;
use crate::shell_window::ShellWindowShared;

const FALLBACK_SCALE_FACTOR: f64 = 1.;

/// Starts the shell with one instance of `application`.
///
/// The instance runs as a task on the shell's tokio runtime, or on the caller's when one is
/// running, which must then be a multi-thread runtime. One instance needs no thread of its own,
/// because the multi-thread worker pool keeps the host's work running beside it (ADR 0009,
/// ADR 0039), so the application's runtime kind applies only in the desktop. The winit event loop
/// runs on the calling thread and blocks until the instance ends. This gives clients the option to
/// run the event loop on the main thread, which some platforms require.
///
/// `font_policy` is the shell's construction input for the instance task's font manager. The
/// instance cannot replace it: faces are only valid inside the manager that issued them, so a
/// live task may not change its face authority (ADR 0005).
pub fn run(application: Application, font_policy: FontPolicy) -> Result<()> {
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

    // Power up a tokio runtime, if none is running yet. The instance, and for an instance that
    // draws into views also the host task, run on it while the event loop blocks this thread, so a
    // caller's runtime needs worker threads.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() != RuntimeFlavor::MultiThread {
            bail!(
                "the shell needs a multi-thread runtime, because its event loop blocks this thread"
            );
        }
        return run_event_loop(application, font_policy);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .on_thread_start(|| {
            platform::set_current_thread_user_interactive("tokio worker");
        })
        .enable_all()
        .build()?;
    let _guard = runtime.enter();
    run_event_loop(application, font_policy)
}

fn run_event_loop(application: Application, font_policy: FontPolicy) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    // Event loop

    let mut winit_context = WinitApplicationHandler::Initializing {
        proxy: event_loop_proxy,
        application: Some(application),
        font_policy,
    };

    info!("Entering event loop");
    event_loop.run_app(&mut winit_context)?;
    info!("Exited event loop");

    let WinitApplicationHandler::Exited { final_result } = winit_context else {
        bail!("Internal error: Exited event loop, but it was never actually exiting");
    };

    final_result
}

/// Tells the event loop that the instance ended, so it exits with the instance's result.
pub(crate) fn report_application_end(proxy: &EventLoopProxy<ShellCommand>, result: Result<()>) {
    if let Err(EventLoopClosed(ShellCommand::ApplicationEnded(result))) =
        proxy.send_event(ShellCommand::ApplicationEnded(result))
    {
        error!("Application ended after the event loop exited: {result:?}");
    }
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
    ToggleFullscreen,
    FullscreenRequested,
    ApplicationEnded(Result<()>),
}

/// ADR: We move the application into the event loop handler.
/// - Because we need to scale_factor() to be passed _to_ application. This does not work on Wayland.
enum WinitApplicationHandler {
    Initializing {
        proxy: EventLoopProxy<ShellCommand>,
        // ADR: Option because we need to move it out.
        // Robustness: use a replace_with variant, so that we don't need an `Option` here.
        application: Option<Application>,
        /// The policy the shell builds the instance task's font manager from (ADR 0005).
        font_policy: FontPolicy,
    },
    Running {
        event_sender: UnboundedSender<InstanceMessage>,
        views: HashMap<WindowId, ViewId>,
    },
    Ended {
        application_result: Result<()>,
    },
    Exited {
        final_result: Result<()>,
    },
}

impl ApplicationHandler<ShellCommand> for WinitApplicationHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initializing {
            proxy,
            application,
            font_policy,
        } = self
        else {
            panic!("Resumed called in an invalid state");
        };

        crate::platform::initialize_platform_menu(proxy.clone());

        let (event_sender, event_receiver) = unbounded_channel();

        let scale_factor = event_loop
            .primary_monitor()
            .map(|pm| pm.scale_factor())
            .unwrap_or_else(|| {
                warn!("Failed to query the current monitor's scale factor, setting to {FALLBACK_SCALE_FACTOR}");
                FALLBACK_SCALE_FACTOR
            });

        // The task boundary is here: `resumed` has the event loop, the font policy and the
        // application in one place, and the contexts must exist before the application starts
        // (ADR 0008).
        let font_manager = match FontManager::new(*font_policy) {
            Ok(font_manager) => font_manager,
            Err(error) => {
                error!("Failed to initialize application font manager: {error:#}");
                *self = Self::Ended {
                    application_result: Err(error),
                };
                event_loop.exit();
                return;
            }
        };
        let startup = Startup {
            proxy: proxy.clone(),
            event_sender: event_sender.downgrade(),
            event_receiver,
            scale_factor,
            font_manager,
        };

        // The application can send CreateWindow as soon as it is spawned, so publish the Running
        // state before starting it.
        let application = application.take().expect("the application is available");
        *self = Self::Running {
            event_sender,
            views: HashMap::new(),
        };
        instance_host::start(application, startup);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellCommand) {
        match event {
            ShellCommand::CreateWindow {
                view_id,
                attributes,
                on_created,
            } => {
                // Window commands cross threads, so one can arrive after the instance's end was
                // reported. The shell is ending then and opens no window.
                let views = match self {
                    Self::Running { views, .. } => views,
                    Self::Initializing { .. } | Self::Ended { .. } | Self::Exited { .. } => {
                        let _ = on_created.send(Err(anyhow!("the shell is ending")));
                        return;
                    }
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
                // A window closed after the instance's end was reported needs no bookkeeping.
                match self {
                    Self::Running { views, .. } => {
                        views.remove(&window.id());
                    }
                    Self::Initializing { .. } | Self::Ended { .. } | Self::Exited { .. } => {}
                }
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
            ShellCommand::ToggleFullscreen => crate::platform::toggle_fullscreen(),
            ShellCommand::FullscreenRequested => {
                // The menu sends this from outside the instance, so it can arrive after the
                // instance's end was reported. No instance receives it then.
                let event_sender = match self {
                    Self::Running { event_sender, .. } => event_sender,
                    Self::Initializing { .. } | Self::Ended { .. } | Self::Exited { .. } => return,
                };
                if let Err(error) = event_sender.send(InstanceMessage::FullscreenRequested) {
                    error!("Failed to send fullscreen request to the application: {error:?}");
                }
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
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event != WindowEvent::RedrawRequested {
            debug!("Received: {:?}", LoggedWindowEvent(&event));
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
            self.send_event(InstanceMessage::View(view_id, event))
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
    fn send_event(&mut self, event: InstanceMessage) {
        let Self::Running { event_sender, .. } = self else {
            error!("Cannot send shell event: application handler must be in the running state.");
            return;
        };

        // The instance's receiver closes when the instance ends, before its end is reported. The
        // report always follows, so the event loop keeps running until it arrives, and only that
        // report ends the loop with the instance's result.
        if event_sender.send(event).is_err() {
            debug!("The instance ended, so it does not receive the window event");
        }
    }
}

/// Time a closure and log its duration.
#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}

/// A window event as the debug log shows it. Key events and IME text are what the user types,
/// passwords at a prompt without echo included, so the log shows only their kind and state
/// (`docs/security.md`).
struct LoggedWindowEvent<'a>(&'a WindowEvent);

impl fmt::Debug for LoggedWindowEvent<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            WindowEvent::KeyboardInput { event, .. } => f
                .debug_struct("KeyboardInput")
                .field("state", &event.state)
                .field("repeat", &event.repeat)
                .finish_non_exhaustive(),
            WindowEvent::Ime(ime) => write!(f, "Ime({})", ime_kind(ime)),
            event => event.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use massive_renderer::ShapingEngineKind;

    use super::*;

    /// The event loop blocks the thread `run` is called on. On a current-thread runtime that is the
    /// runtime's only thread, so the instance would never run, and the shell must refuse instead of
    /// hanging.
    #[test]
    fn run_refuses_a_current_thread_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let application = Application::new("never runs", |_| async { Ok(()) });

        let result = runtime.block_on(async {
            run(
                application,
                FontPolicy::bare(ShapingEngineKind::available()[0]),
            )
        });

        let error = result.unwrap_err();
        assert!(error.to_string().contains("multi-thread"), "{error}");
    }
}
