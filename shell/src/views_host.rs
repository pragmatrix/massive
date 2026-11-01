//! The shell's host for an instance that presents views
//! ([`DrawTarget::Views`](massive_applications::DrawTarget::Views)). It draws the instance's
//! one view into a window of the view's size, so an application written for the desktop also runs
//! on its own.

use anyhow::{Result, bail};
use log::{debug, warn};
use tokio::select;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
use winit::event_loop::EventLoopProxy;

use crate::instance_windows::ShellWindows;
use crate::shell::{
    ShellCommand, Startup, rendering_task_context, report_application_end, spawn_instance_thread,
};
use crate::{AsyncWindowRenderer, ShellWindow};

/// Starts the host task, which starts the instance once the instance's root location exists in the
/// host's scene.
///
/// The instance collects instance changes on its own thread. The host task collects the scene
/// changes among them and renders them, like the desktop does for its instances.
pub fn start(application: Application, startup: Startup) -> Result<()> {
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
        let result = host(
            application,
            instance_task_context,
            windows,
            event_receiver,
            scale_factor,
            proxy.clone(),
        )
        .await;
        // The instance thread reports the instance's end, so only the host's failure is reported
        // here.
        if result.is_err() {
            report_application_end(&proxy, result);
        }
    }));
    Ok(())
}

async fn host(
    application: Application,
    instance_task_context: TaskContext,
    windows: ShellWindows,
    mut shell_events: UnboundedReceiver<InstanceMessage>,
    scale_factor: f64,
    proxy: EventLoopProxy<ShellCommand>,
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
    spawn_instance_thread(
        application.runtime_kind(),
        application.run(context),
        instance_task_context,
        proxy,
    )?;

    let mut host = ViewsHost {
        instance_id,
        windows,
        instance_events,
        view: None,
    };

    loop {
        select! {
            output = outputs.recv() => {
                // The channel closes when the instance's context is gone, which is after its last
                // submission.
                let Some((_, output)) = output else {
                    return Ok(());
                };
                host.apply_output(output).await?;
            }
            Some(message) = shell_events.recv() => host.forward(message)?,
        }
    }
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
    async fn apply_output(&mut self, output: InstanceOutput) -> Result<()> {
        let InstanceOutput::Submission(submission) = output else {
            unreachable!("the shell never calls tools, so no tool result arrives");
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
                self.view = None;
            }
            InstanceChange::Configuration(request) => {
                warn!("The shell has no desktop configuration, so it drops {request:?}");
            }
            // The host keeps the root until it ends.
            InstanceChange::End(_) => {}
        }
        Ok(())
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
