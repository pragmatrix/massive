use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use anyhow::Result;
use anyhow::{Context, anyhow, bail};
use log::{debug, error, info, warn};
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, WeakUnboundedSender, unbounded_channel,
};
use tokio::sync::oneshot;

use wgpu::{Surface, SurfaceTarget};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopClosed, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::task_context::TaskContext;
use massive_applications::{
    Application, ApplicationMessage, CreationMode, InstanceContext, InstanceEnvironment,
    InstanceFuture, InstanceId, Rendering, RuntimeKind, ViewEvent, ViewId, run_instance,
};
use massive_renderer::{FontManager, FontPolicy};
use massive_scene::{AnyCollector, SceneChange};

use crate::instance_windows::ShellWindows;
use crate::platform;
use crate::shell_window::ShellWindowShared;
use crate::views_host;

const FALLBACK_SCALE_FACTOR: f64 = 1.;
/// The OS thread name of the shell's instance.
const INSTANCE_THREAD_NAME: &str = "massive instance";

/// Starts the shell with one instance of `application`.
///
/// The instance runs on a dedicated thread under the runtime its application names (ADR 0009). The
/// winit event loop runs on the calling thread and blocks until the instance ends. This gives
/// clients the option to run the event loop on the main thread, which some platforms require.
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

    // Power up a tokio runtime, if none is running yet.

    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            // Already inside a Tokio runtime.
            run_with_tokio(application, font_policy)
        }
        Err(_) => {
            // Create and enter a multi-thread runtime so tokio::spawn can run while the event loop blocks.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .on_thread_start(|| {
                    platform::set_current_thread_user_interactive("tokio worker");
                })
                .enable_all()
                .build()?;
            let _guard = runtime.enter();
            let r = run_with_tokio(application, font_policy);
            drop(_guard);
            r
        }
    }
}

fn run_with_tokio(application: Application, font_policy: FontPolicy) -> Result<()> {
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

/// The contexts of the application task: its change queue, animation clock, movement runtime
/// and a shaping context over the supplied font manager (ADR 0008).
pub(crate) fn application_task_context(font_manager: FontManager) -> TaskContext {
    TaskContext::new(
        AnyCollector::for_type::<SceneChange>(),
        AnimationCoordinator::new(),
        MovementRuntime::default(),
        font_manager.new_shaping_context(),
    )
}

/// What the shell hands to its instance's start once the event loop runs.
pub(crate) struct Startup {
    pub(crate) proxy: EventLoopProxy<ShellCommand>,
    pub(crate) event_sender: WeakUnboundedSender<ApplicationMessage>,
    pub(crate) event_receiver: UnboundedReceiver<ApplicationMessage>,
    pub(crate) scale_factor: f64,
    pub(crate) font_manager: FontManager,
}

/// Starts the shell's single instance.
fn start_instance(application: Application, startup: Startup) -> Result<()> {
    match application.rendering() {
        Rendering::Instance => start_instance_drawing_windows(application, startup),
        Rendering::Host => views_host::start(application, startup),
    }
}

/// Starts an instance that opens its own windows and renders scene changes into them, so its task
/// collects scene changes like the desktop's.
fn start_instance_drawing_windows(application: Application, startup: Startup) -> Result<()> {
    let Startup {
        proxy,
        event_sender,
        event_receiver,
        scale_factor,
        font_manager,
    } = startup;
    let windows = ShellWindows::new(proxy.clone(), event_sender);
    // Nothing receives the instance's output: an instance that draws its own windows sends none.
    let (output_sender, _) = unbounded_channel();
    let environment = InstanceEnvironment::new(output_sender, scale_factor, None);
    let context = InstanceContext::drawing_windows(
        InstanceId::new(),
        CreationMode::New(Default::default()),
        environment,
        Arc::new(windows),
        event_receiver,
    );
    spawn_instance_thread(
        application.runtime_kind(),
        application.run(context),
        application_task_context(font_manager),
        proxy,
    )
}

/// Runs the instance on a dedicated thread (ADR 0009) at the user-interactive priority that the
/// shell gives its application, and reports the instance's end to the event loop.
pub(crate) fn spawn_instance_thread(
    runtime_kind: RuntimeKind,
    instance: InstanceFuture,
    task_context: TaskContext,
    proxy: EventLoopProxy<ShellCommand>,
) -> Result<()> {
    thread::Builder::new()
        .name(INSTANCE_THREAD_NAME.into())
        .spawn(move || {
            platform::set_current_thread_user_interactive(INSTANCE_THREAD_NAME);
            let mut runtime = runtime_kind.runtime_builder();
            runtime.on_thread_start(|| {
                platform::set_current_thread_user_interactive("instance worker");
            });
            let result = run_instance(runtime, instance, task_context);
            report_application_end(&proxy, result);
        })
        .context("Spawning the instance thread")?;
    Ok(())
}

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
        /// The policy the shell builds the application task's manager from (ADR 0005).
        font_policy: FontPolicy,
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
        if let Err(error) = start_instance(application, startup) {
            error!("Failed to start the application: {error:#}");
            *self = Self::Ended {
                application_result: Err(error),
            };
            event_loop.exit();
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
            ShellCommand::ToggleFullscreen => crate::platform::toggle_fullscreen(),
            ShellCommand::FullscreenRequested => {
                let Self::Running { event_sender, .. } = self else {
                    panic!(
                        "Received FullscreenRequested while WinitApplicationHandler is not Running"
                    );
                };
                if let Err(error) = event_sender.send(ApplicationMessage::FullscreenRequested) {
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

/// Time a closure and log its duration.
#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}
