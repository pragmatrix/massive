use std::{
    mem,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use log::{error, info};
use parking_lot::Mutex;
use tokio::sync::mpsc::WeakUnboundedSender;
use winit::{event, window::WindowId};

use massive_geometry::{Color, Matrix4, SizePx};
use massive_renderer::{RenderGeometry, RenderPacing, RenderSubmission, RenderTarget};

use crate::{
    ShellEvent,
    render_thread::{RenderThreadSubmission, RendererMessage, render_thread},
    window_renderer::WindowRenderer,
};

#[derive(Debug)]
pub struct AsyncWindowRenderer {
    window_id: WindowId,
    msg_sender: Sender<RendererMessage>,
    thread_handle: Option<JoinHandle<()>>,
    geometry: RenderGeometry,
    /// All current changes (taken out when the renderer processes Submit)
    ///
    /// This is not transferred together with Submit, so that renderer can pick it up as fast as
    /// possible. But it is meant as a "visual" transaction of one or more consistent frame changes.
    submission: Arc<Mutex<RenderThreadSubmission>>,
}

impl AsyncWindowRenderer {
    // Architecture: Camera does not feel to belong here. It already moved from the Renderer to here.
    pub fn new(
        window_renderer: WindowRenderer,
        geometry: RenderGeometry,
        shell_events: WeakUnboundedSender<ShellEvent>,
    ) -> Self {
        let id = window_renderer.window_id();
        let view_projection = geometry.view_projection();

        let (msg_sender, msg_receiver) = mpsc::channel();

        let submission = RenderThreadSubmission::new(view_projection);
        let submission = Arc::new(Mutex::new(submission));
        let submission2 = submission.clone();

        let thread_handle = thread::spawn(move || {
            match render_thread(msg_receiver, window_renderer, submission2, shell_events) {
                Ok(()) => {
                    info!("Render loop ended because the sender disconnected");
                }
                Err(e) => {
                    error!("Render loop ended because of an error: {e:?}");
                }
            }
        });

        Self {
            window_id: id,
            msg_sender,
            thread_handle: Some(thread_handle),
            geometry,
            submission,
        }
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn geometry(&self) -> &RenderGeometry {
        &self.geometry
    }

    pub fn resize_redraw(&mut self, rrr: impl Into<ResizeRedrawRequest>) -> Result<()> {
        let rrr = rrr.into();

        if let Some(window) = rrr.window
            && window != self.window_id
        {
            return Ok(());
        }

        match rrr.mode {
            ResizeRedrawMode::Resize(wh) => self.resize_and_redraw(wh),
            ResizeRedrawMode::Redraw => {
                // Robustness: Do this only in fast pacing mode?
                //
                // Robustness: This does not seem necessary on macOS at all. The explicit redraw
                // requests seem to be sent only after a resize event, which we handle above. I've
                // tested sleep / minimizing, etc.
                self.post_msg(RendererMessage::Redraw)
            }
            ResizeRedrawMode::None => Ok(()),
        }
    }

    fn resize_and_redraw(&mut self, surface_size: SizePx) -> Result<()> {
        self.geometry.set_surface_size(surface_size);

        // Robustness: May defer the view projection update until the frame that is ensured to be
        // rendered after the Resize? This submission update could be picked up before the render
        // thread receives the Resize and the follow up redraw.
        self.submission.lock().view_projection = self.geometry.view_projection();
        self.post_msg(RendererMessage::Resize(surface_size))?;

        // We immediately issue a redraw after a resize. Usually, when a WindowEvent is used
        // to generate Resize / Redraw requests, another redraw will follow, but this takes
        // too long for resizing.
        //
        // Robustness: Do this only in fast pacing mode?
        self.post_msg(RendererMessage::Redraw)
    }

    pub fn set_background_color(&self, color: Option<Color>) -> Result<()> {
        self.post_msg(RendererMessage::SetBackgroundColor(color))
    }

    fn post_msg(&self, message: RendererMessage) -> Result<()> {
        self.msg_sender
            .send(message)
            .context("Sending renderer message")?;
        Ok(())
    }
}

impl Drop for AsyncWindowRenderer {
    fn drop(&mut self) {
        // Explicitly drop the sender first to close the channel
        // This will cause the receiving thread to exit
        mem::drop(mem::replace(&mut self.msg_sender, mpsc::channel().0));

        // Then join the thread to ensure clean shutdown
        if let Some(handle) = self.thread_handle.take()
            && let Err(e) = handle.join()
        {
            error!("Error joining AsyncWindowRenderer thread: {e:?}");
        }
    }
}

impl RenderTarget for AsyncWindowRenderer {
    fn render(&mut self, render_submission: RenderSubmission) -> Result<()> {
        // Push the changes _directly_ to the renderer which picks it up in the next redraw. This
        // may asynchronously overtake the subsequent redraw / resize requests if a previous one is
        // currently on its way.

        if let Some(camera) = render_submission.camera_update {
            self.geometry.set_camera(camera);
        }

        let submission = RenderThreadSubmission {
            changes: render_submission.changes,
            present_mode: match render_submission.pacing {
                RenderPacing::Fast => wgpu::PresentMode::AutoNoVsync,
                RenderPacing::Smooth => wgpu::PresentMode::AutoVsync,
            },
            view_projection: self.geometry.view_projection(),
        };

        self.submission.lock().accumulate(submission);

        // Performance: We don't know exactly if shall submit also in Smooth mode here (because the
        // thread is driving). But we keep this for now, because we don't have the _effective_ frame
        // pacing available here I guess.

        self.post_msg(RendererMessage::Redraw)
    }
}

#[derive(Debug, Default)]
pub struct ResizeRedrawRequest {
    window: Option<WindowId>,
    mode: ResizeRedrawMode,
}

/// Request to the renderer to resize and / or redraw.
#[derive(Debug, Default)]
pub enum ResizeRedrawMode {
    Resize(SizePx),
    Redraw,
    #[default]
    None,
}

impl From<&event::WindowEvent> for ResizeRedrawRequest {
    fn from(window_event: &event::WindowEvent) -> Self {
        use event::WindowEvent;
        let mode = match window_event {
            WindowEvent::Resized(physical_size) => {
                ResizeRedrawMode::Resize((physical_size.width, physical_size.height).into())
            }
            WindowEvent::RedrawRequested => ResizeRedrawMode::Redraw,
            _ => ResizeRedrawMode::None,
        };

        ResizeRedrawRequest { window: None, mode }
    }
}

impl From<&ShellEvent> for ResizeRedrawRequest {
    fn from(value: &ShellEvent) -> Self {
        match value {
            ShellEvent::WindowEvent(window_id, window_event) => {
                let mut request: ResizeRedrawRequest = window_event.into();
                request.window = (*window_id).into();
                request
            }
            _ => Self::default(),
        }
    }
}
