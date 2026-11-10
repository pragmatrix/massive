//! The shell's one instance: its start, its task, and for an instance that draws into views the
//! host task that draws its one view into a window of the view's size (ADR 0039).

use std::mem;
use std::sync::Arc;

use anyhow::{Result, bail};
use log::{debug, warn};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::{select, task};

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::prelude::*;
use massive_applications::task_context::{self, TaskContext};
use massive_applications::{
    Application, CreationMode, DrawTarget, InstanceChange, InstanceContext, InstanceEnvironment,
    InstanceFuture, InstanceId, InstanceMessage, InstanceOutput, ViewChange, ViewCreationInfo,
    ViewEvent, ViewId, begin_frame,
};
use massive_geometry::{BoxPx, Rect};
use massive_renderer::FontManager;
use massive_scene::prelude::*;
use massive_scene::{AnyCollector, SceneChange};
use tokio::sync::mpsc::WeakUnboundedSender;
use winit::event_loop::EventLoopProxy;

use crate::instance_windows::ShellWindows;
use crate::shell::{ShellCommand, report_application_end};
use crate::{AsyncWindowRenderer, ShellWindow};

/// The contexts of a task that renders scene changes itself: an instance that renders into its own
/// windows, or the host task of an instance that draws into views. They are its change queue,
/// animation clock, movement runtime and a shaping context over the supplied font manager
/// (ADR 0008).
fn rendering_task_context(font_manager: FontManager) -> TaskContext {
    TaskContext::new(
        AnyCollector::for_type::<SceneChange>(),
        AnimationCoordinator::new(),
        MovementRuntime::default(),
        font_manager.new_shaping_context(),
    )
}

/// What the shell hands to its instance's start once the event loop runs.
pub(crate) struct Startup {
    pub proxy: EventLoopProxy<ShellCommand>,
    pub event_sender: WeakUnboundedSender<InstanceMessage>,
    pub event_receiver: UnboundedReceiver<InstanceMessage>,
    pub scale_factor: f64,
    pub font_manager: FontManager,
}

/// Starts the shell's single instance, following where its application draws.
pub(crate) fn start(application: Application, startup: Startup) {
    match application.draw_target() {
        DrawTarget::Windows => start_drawing_windows(application, startup),
        DrawTarget::Views => start_drawing_views(application, startup),
    }
}

/// Starts an instance that opens its own windows and renders scene changes into them, so its task
/// collects scene changes like the desktop's.
fn start_drawing_windows(application: Application, startup: Startup) {
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
    // The instance owns its windows, so they are closed before its task reports the end.
    spawn_instance::<SceneChange>(
        application.run(context),
        rendering_task_context(font_manager),
        move |result| report_application_end(&proxy, result),
    );
}

/// Runs the instance as a task on the shell's runtime under its task context (ADR 0008), and hands
/// the instance's result to `report_end`. `C` is the change type of the instance's queue.
fn spawn_instance<C>(
    instance: InstanceFuture,
    task_context: TaskContext,
    report_end: impl FnOnce(Result<()>) + Send + 'static,
) where
    C: From<SceneChange> + std::fmt::Debug + Send + 'static,
{
    tokio::spawn(async move {
        let (result, final_changes) =
            task_context::with_context_and_final_changes::<C, _>(task_context, instance).await;
        // The shell ends with its one instance, so nothing renders what the instance's task
        // changed after its final submission (ADR 0040).
        drop(final_changes.release());
        report_end(result);
    });
}

/// Starts an instance that draws into views. The host task starts the instance once the instance's
/// root location exists in the host's scene.
///
/// The instance collects instance changes in its own task. The host task collects the scene
/// changes among them and renders them, like the desktop does for its instances.
fn start_drawing_views(application: Application, startup: Startup) {
    let Startup {
        proxy,
        event_sender,
        event_receiver,
        scale_factor,
        font_manager,
    } = startup;
    // Both tasks shape with the one font manager, because faces are only valid inside the manager
    // that issued them (ADR 0005).
    let instance_task_context = TaskContext::new(
        AnyCollector::for_type::<InstanceChange>(),
        AnimationCoordinator::new(),
        MovementRuntime::default(),
        font_manager.new_shaping_context(),
    );
    let host_task_context = rendering_task_context(font_manager);
    let windows = ShellWindows::new(proxy.clone(), event_sender);

    tokio::spawn(task_context::with_context(host_task_context, async move {
        let mut shell_events = event_receiver;
        let result = host(
            application,
            instance_task_context,
            windows,
            &mut shell_events,
            scale_factor,
        )
        .await;
        // The shell's events stay received until the end is reported, so the view's renderer can
        // still deliver its ticks until the view closes.
        report_application_end(&proxy, result);
    }));
}

/// Runs the instance and serves it until it ends. The host is the only party that reports the end,
/// and it closes the view's window first, so the event loop handles the window's destruction
/// before it ends.
async fn host(
    application: Application,
    instance_task_context: TaskContext,
    windows: ShellWindows,
    shell_events: &mut UnboundedReceiver<InstanceMessage>,
    scale_factor: f64,
) -> Result<()> {
    // The root stays alive until the host ends, so the instance's views keep their parent while it
    // is served.
    let (_root_transform, root) = identity_location().submit();

    let instance_id = InstanceId::new();
    let (instance_events, instance_events_receiver) = unbounded_channel();
    let (output_sender, mut outputs) = unbounded_channel();
    let context = InstanceContext::new(
        instance_id,
        CreationMode::New(Default::default()),
        InstanceEnvironment::new(output_sender, scale_factor, None),
        root.to_ref(),
        instance_events_receiver,
    );
    let (end_sender, mut instance_end) = oneshot::channel();
    spawn_instance::<InstanceChange>(
        application.run(context),
        instance_task_context,
        move |result| {
            // After a failure of the host, nothing waits for the instance's result anymore.
            let _ = end_sender.send(result);
        },
    );

    let mut host = InstanceHost {
        instance_id,
        windows,
        instance_events,
        view: ViewSlot::Awaiting,
    };

    let result = match host
        .serve(&mut outputs, shell_events, &mut instance_end)
        .await
    {
        ServeEnd::InstanceEnded(result) => result,
        ServeEnd::HostFailed(error) => {
            host.request_shutdown();
            Err(error)
        }
    };
    // The view's window closes before `start` reports the end.
    host.close_view();
    result
}

/// How serving an instance ended.
#[derive(Debug)]
enum ServeEnd {
    /// The instance ended with this result.
    InstanceEnded(Result<()>),
    /// The host failed while the instance may still run.
    HostFailed(anyhow::Error),
}

#[derive(Debug)]
struct InstanceHost {
    instance_id: InstanceId,
    windows: ShellWindows,
    instance_events: UnboundedSender<InstanceMessage>,
    /// The instance's one view.
    view: ViewSlot,
}

/// The shell's one view per instance.
///
/// A view the instance creates after its first one closed is refused, because its new window's
/// renderer would miss the scene changes the first renderer already consumed.
#[derive(Debug)]
enum ViewSlot {
    /// The instance has not created its view yet. Its scene changes wait in the host's queue.
    Awaiting,
    Open(Box<HostedView>),
    Closed,
}

impl InstanceHost {
    /// Serves the instance until its task reports its result.
    ///
    /// The outputs that are still queued when the result arrives are applied before serving ends,
    /// so the instance's last submission is drawn and its views are destroyed.
    async fn serve(
        &mut self,
        outputs: &mut UnboundedReceiver<(InstanceId, InstanceOutput)>,
        shell_events: &mut UnboundedReceiver<InstanceMessage>,
        instance_end: &mut oneshot::Receiver<Result<()>>,
    ) -> ServeEnd {
        let instance_result = loop {
            let step = select! {
                Some((_, output)) = outputs.recv() => self.apply_output(output).await,
                Some(message) = shell_events.recv() => self.forward(message),
                // An instance that panics aborts the process (`panic = "abort"`), so a running
                // shell always receives the instance's result.
                result = &mut *instance_end => {
                    break result.expect("the instance task always reports its result");
                }
            };
            if let Err(error) = step {
                return ServeEnd::HostFailed(error);
            }
        };
        while let Ok((_, output)) = outputs.try_recv() {
            if let Err(error) = self.apply_output(output).await {
                return ServeEnd::HostFailed(error);
            }
        }
        ServeEnd::InstanceEnded(instance_result)
    }

    /// Asks the instance to end after the host failed. The shell ends with the host's error and
    /// does not wait for the instance.
    fn request_shutdown(&self) {
        if self
            .instance_events
            .send(InstanceMessage::Shutdown(self.instance_id))
            .is_err()
        {
            debug!("The instance ended before the host failed");
        }
    }

    async fn apply_output(&mut self, output: InstanceOutput) -> Result<()> {
        let submission = match output {
            InstanceOutput::Submission(submission) => submission,
            InstanceOutput::ToolResult { .. } => {
                bail!("the instance answered a tool call, but the shell never calls tools")
            }
        };
        let (changes, pacing) = submission.into_parts();
        for change in changes.release() {
            self.apply_change(change).await?;
        }
        match &mut self.view {
            ViewSlot::Open(view) => begin_frame()
                .render_submission()
                .with_pacing(pacing)
                .submit_to(&mut view.renderer)?,
            // Before the view opens, the scene changes wait in the host's queue until its window
            // renders them. After it closed, the shell ends with the instance.
            ViewSlot::Awaiting | ViewSlot::Closed => {}
        }
        Ok(())
    }

    async fn apply_change(&mut self, change: InstanceChange) -> Result<()> {
        match change {
            InstanceChange::Scene(change) => submit(change),
            InstanceChange::CreateView(info) => {
                match self.view {
                    ViewSlot::Awaiting => {}
                    ViewSlot::Open(_) | ViewSlot::Closed => bail!(
                        "the shell hosts one view per instance, but the instance created another"
                    ),
                }
                self.view = ViewSlot::Open(Box::new(HostedView::open(&self.windows, &info).await?));
            }
            InstanceChange::View(id, change) => self.hosted_view(id)?.apply(change),
            InstanceChange::DestroyView(id) => {
                self.hosted_view(id)?;
                self.close_view();
            }
            InstanceChange::Configuration(request) => {
                warn!("The shell has no desktop configuration, so it drops {request:?}");
            } // The host keeps the root until it ends.
        }
        Ok(())
    }

    /// Closes the view's window, if the view is open.
    ///
    /// Dropping the view's renderer joins its render thread, which can take up to a frame. The
    /// join runs in `block_in_place`, so the worker's other tasks, such as the instance, move to
    /// another worker meanwhile.
    fn close_view(&mut self) {
        match mem::replace(&mut self.view, ViewSlot::Closed) {
            ViewSlot::Open(view) => task::block_in_place(|| drop(view)),
            ViewSlot::Awaiting | ViewSlot::Closed => {}
        }
    }

    fn hosted_view(&mut self, id: ViewId) -> Result<&mut HostedView> {
        match &mut self.view {
            ViewSlot::Open(view) if view.id == id => Ok(view),
            ViewSlot::Open(_) | ViewSlot::Awaiting | ViewSlot::Closed => {
                bail!("the instance changed the view {id:?}, which it never created")
            }
        }
    }

    /// Forwards an event from the shell to the instance. A close request becomes the shutdown
    /// request, as the desktop sends it to an instance that it closes.
    fn forward(&mut self, message: InstanceMessage) -> Result<()> {
        let message = match message {
            InstanceMessage::View(_, ViewEvent::CloseRequested) => {
                InstanceMessage::Shutdown(self.instance_id)
            }
            InstanceMessage::View(view_id, event) => {
                match &mut self.view {
                    ViewSlot::Open(view) if view.id == view_id => {
                        view.renderer.resize_redraw(&event)?
                    }
                    ViewSlot::Open(_) | ViewSlot::Awaiting | ViewSlot::Closed => {}
                }
                InstanceMessage::View(view_id, event)
            }
            message @ (InstanceMessage::ApplyAnimations(_)
            | InstanceMessage::FullscreenRequested
            | InstanceMessage::Shutdown(_)
            | InstanceMessage::Tool(_)) => message,
        };
        if self.instance_events.send(message).is_err() {
            debug!("The instance ended, so it does not receive the shell's event");
        }
        Ok(())
    }
}

/// A view of the instance and the window that shows it.
#[derive(Debug)]
struct HostedView {
    id: ViewId,
    window: ShellWindow,
    renderer: AsyncWindowRenderer,
    /// Whether the IME is allowed, so it is only toggled when the view's input area appears or
    /// disappears. Disallowing it discards a pending composition.
    ime_allowed: bool,
}

impl HostedView {
    async fn open(windows: &ShellWindows, info: &ViewCreationInfo) -> Result<Self> {
        let window = windows.new_window(info.id, info.size()).await?;
        let renderer = window
            .renderer()
            .with_shapes()
            .with_text()
            .with_images()
            .build()
            .await?;
        Ok(Self {
            id: info.id,
            window,
            renderer,
            ime_allowed: false,
        })
    }

    fn apply(&mut self, change: ViewChange) {
        match change {
            ViewChange::Resize(extent) => {
                warn!("The shell does not resize a view's window, so it ignores {extent:?}");
            }
            ViewChange::SetTitle(title) => self.window.set_title(&title),
            ViewChange::SetCursor(cursor) => self.window.set_cursor(cursor),
            ViewChange::SetImeArea(area) => {
                if self.ime_allowed != area.is_some() {
                    self.ime_allowed = area.is_some();
                    self.window.set_ime_allowed(self.ime_allowed);
                }
                // The window is as large as the view, so view-local coordinates are window pixels.
                if let Some(area) = area {
                    self.window.set_ime_cursor_area(window_pixels(area));
                }
            }
        }
    }
}

fn window_pixels(area: Rect) -> BoxPx {
    let area = area.rounded();
    BoxPx::new(
        (area.left as i32, area.top as i32).into(),
        (area.right as i32, area.bottom as i32).into(),
    )
}
