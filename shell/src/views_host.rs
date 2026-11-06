//! The shell's host for an instance that presents views
//! ([`DrawTarget::Views`](massive_applications::DrawTarget::Views)). It draws the instance's
//! one view into a window of the view's size, so an application written for the desktop also runs
//! on its own.

use anyhow::{Result, bail};
use log::{debug, warn};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::{select, task};

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::prelude::*;
use massive_applications::task_context::{self, TaskContext};
use massive_applications::{
    Application, CreationMode, InstanceChange, InstanceContext, InstanceEnvironment, InstanceId,
    InstanceMessage, InstanceOutput, ViewChange, ViewCreationInfo, ViewEvent, ViewId, begin_frame,
};
use massive_geometry::{BoxPx, Rect};
use massive_scene::AnyCollector;
use massive_scene::prelude::*;

use crate::instance_windows::ShellWindows;
use crate::shell::{Startup, rendering_task_context, report_application_end, spawn_instance};
use crate::{AsyncWindowRenderer, ShellWindow};

/// Starts the host task, which starts the instance once the instance's root location exists in the
/// host's scene.
///
/// The instance collects instance changes in its own task. The host task collects the scene
/// changes among them and renders them, like the desktop does for its instances.
pub fn start(application: Application, startup: Startup) {
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
        // The shell's events stay received until the end is reported, because a send to a closed
        // queue ends the event loop without the instance's result.
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
    // The root stays alive until the host ends, which is after the instance ended.
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
    spawn_instance(
        application.run(context),
        instance_task_context,
        move |result| {
            // After a failure of the host, nothing waits for the instance's result anymore.
            let _ = end_sender.send(result);
        },
    );

    let mut host = ViewsHost {
        instance_id,
        windows,
        instance_events,
        view: None,
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
struct ViewsHost {
    instance_id: InstanceId,
    windows: ShellWindows,
    instance_events: UnboundedSender<InstanceMessage>,
    /// The instance's one view, once the instance created it.
    view: Option<HostedView>,
}

impl ViewsHost {
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
        let InstanceOutput::Submission(submission) = output else {
            bail!("the instance answered a tool call, but the shell never calls tools");
        };
        let (changes, pacing) = submission.into_parts();
        for change in changes.release() {
            self.apply_change(change).await?;
        }
        // Without a view, the scene changes wait in the host's queue until the view's window
        // renders them.
        if let Some(view) = &mut self.view {
            begin_frame()
                .render_submission()
                .with_pacing(pacing)
                .submit_to(&mut view.renderer)?;
        }
        Ok(())
    }

    async fn apply_change(&mut self, change: InstanceChange) -> Result<()> {
        match change {
            InstanceChange::Scene(change) => submit(change),
            InstanceChange::CreateView(info) => {
                if self.view.is_some() {
                    bail!("the shell hosts one view, but the instance created another one");
                }
                self.view = Some(HostedView::open(&self.windows, &info).await?);
            }
            InstanceChange::View(id, change) => self.hosted_view(id)?.apply(change),
            InstanceChange::DestroyView(id) => {
                self.hosted_view(id)?;
                self.close_view();
            }
            InstanceChange::Configuration(request) => {
                warn!("The shell has no desktop configuration, so it drops {request:?}");
            }
            // The host keeps the root until it ends.
            InstanceChange::End(_) => {}
        }
        Ok(())
    }

    /// Closes the view's window, if the instance has a view.
    ///
    /// Dropping the view's renderer joins its render thread, which can take up to a frame. The
    /// join runs in `block_in_place`, so the worker's other tasks, such as the instance, move to
    /// another worker meanwhile.
    fn close_view(&mut self) {
        if let Some(view) = self.view.take() {
            task::block_in_place(|| drop(view));
        }
    }

    fn hosted_view(&mut self, id: ViewId) -> Result<&mut HostedView> {
        match &mut self.view {
            Some(view) if view.id == id => Ok(view),
            _ => bail!("the instance changed the view {id:?}, which it never created"),
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
                if let Some(view) = &mut self.view
                    && view.id == view_id
                {
                    view.renderer.resize_redraw(&event)?;
                }
                InstanceMessage::View(view_id, event)
            }
            message => message,
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
