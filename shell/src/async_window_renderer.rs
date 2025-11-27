use std::{
    mem,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result, anyhow};
use log::{error, info, warn};
use massive_applications::{RenderPacing, RenderTarget};
use tokio::sync::mpsc::WeakUnboundedSender;
use winit::{event, window::WindowId};

use crate::{ShellEvent, message_filter::keep_last_per_variant, window_renderer::WindowRenderer};
use massive_geometry::{Camera, Color};
use massive_renderer::RenderGeometry;
use massive_scene::{ChangeCollector, Matrix, SceneChanges};

#[derive(Debug)]
pub struct AsyncWindowRenderer {
    window_id: WindowId,
    // For pushing changes directly to the renderer.
    change_collector: Arc<ChangeCollector>,
    msg_sender: Sender<RendererMessage>,
    thread_handle: Option<JoinHandle<()>>,
    geometry: RenderGeometry,
    /// The current render pacing as seen from the client. This may not reflect reality, as it is
    /// synchronized with the renderer asynchronously.
    pacing: RenderPacing,
}

#[derive(Debug)]
enum RendererMessage {
    Resize((u32, u32)),
    Redraw { view_projection: Matrix },
    SetPresentMode(wgpu::PresentMode),
    SetBackgroundColor(Option<Color>),
    // Protocol: When adding a new RenderMessage, consider filter_latest_messages().
}

impl AsyncWindowRenderer {
    // Architecture: Camera does not feel to belong here. It already moved from the Renderer to here.
    pub fn new(
        window_renderer: WindowRenderer,
        geometry: RenderGeometry,
        shell_events: Option<WeakUnboundedSender<ShellEvent>>,
    ) -> Self {
        let id = window_renderer.window_id();
        let change_collector = window_renderer.change_collector().clone();
        let view_projection = geometry.view_projection();

        let (msg_sender, msg_receiver) = mpsc::channel();

        let thread_handle = thread::spawn(move || {
            if let Err(e) =
                Self::render_loop(msg_receiver, window_renderer, shell_events, view_projection)
            {
                error!("Render loop crashed with {e:?}");
            }
        });

        Self {
            window_id: id,
            change_collector,
            msg_sender,
            thread_handle: Some(thread_handle),
            geometry,
            pacing: Default::default(),
        }
    }

    fn render_loop(
        msg_receiver: mpsc::Receiver<RendererMessage>,
        mut window_renderer: WindowRenderer,
        shell_events: Option<WeakUnboundedSender<ShellEvent>>,
        mut view_projection: Matrix,
    ) -> Result<()> {
        let mut messages = Vec::new();

        loop {
            // Detail: Because the previous event may take some time to process, there might be some
            // additional messages pending, but we don't pull them to avoid getting into a situation
            // in smooth rendering that there is never a rendering.

            let smooth = window_renderer.present_mode() == wgpu::PresentMode::AutoVsync;
            if messages.is_empty() {
                // blocking path.
                if smooth {
                    // smooth. This may block.
                    Self::render_frame(&mut window_renderer, &shell_events, &view_projection)?;
                } else {
                    // fast mode.
                    Self::wait_for_events(&msg_receiver, &mut messages)?;
                }
            }

            // Non-blocking.
            Self::retrieve_pending_events(&msg_receiver, &mut messages)?;
            messages = keep_last_per_variant(messages, |_| true);

            if messages.is_empty() {
                continue;
            }
            // Detail: I think there could only be 4 events in there because of keep_last_per_variant?
            match messages.remove(0) {
                RendererMessage::Resize(new_size) => {
                    // Optimization: If we resize and change present mode the same time, we would only need to reconfigure
                    // the surface once. Renderer might even reconfigure lazily.
                    window_renderer.resize(new_size);
                }
                RendererMessage::SetPresentMode(present_mode) => {
                    // Detail: Switching from NoVSync to VSync takes ~200 microseconds,
                    // from VSync to NoVSync around ~2.7 milliseconds (measured in the logs example --release).
                    window_renderer.set_present_mode(present_mode);
                }
                RendererMessage::Redraw {
                    view_projection: new_view_projection,
                } => {
                    view_projection = new_view_projection;
                    // In smooth mode, we ignore explicit redraw requests.
                    if !smooth {
                        Self::render_frame(&mut window_renderer, &shell_events, &view_projection)?;
                    } else {
                        warn!("Explicit redraw in smooth rendering mode");
                    }
                }
                RendererMessage::SetBackgroundColor(color) => {
                    window_renderer.set_background_color(color);
                }
            }
        }
    }

    /// Wait until events are available. Blocks if none available.
    ///
    /// Blocks until at least one event is available.
    fn wait_for_events(
        msg_receiver: &mpsc::Receiver<RendererMessage>,
        events: &mut Vec<RendererMessage>,
    ) -> Result<()> {
        if events.is_empty() {
            events.push(msg_receiver.recv()?);
        }
        Ok(())
    }

    /// Retrieve all pending events. Non blocking.
    fn retrieve_pending_events(
        msg_receiver: &mpsc::Receiver<RendererMessage>,
        events: &mut Vec<RendererMessage>,
    ) -> Result<()> {
        while let Ok(event) = msg_receiver.try_recv() {
            events.push(event);
        }
        Ok(())
    }

    fn render_frame(
        renderer: &mut WindowRenderer,
        apply_animations_to: &Option<WeakUnboundedSender<ShellEvent>>,
        view_projection: &Matrix,
    ) -> Result<()> {
        // Detail: In VSync presentation mode, this blocks until the next VSync beginning
        // with the second frame after that. Therefore we apply scene changes afterwards.
        // This improves time of first change to render time considerably.
        let texture = renderer.get_next_texture()?;

        // Detail: Presentation timestamps are only sent when the presentation waited for a VSync.
        if let Some(apply_animations_to) = apply_animations_to
            && renderer.present_mode() == wgpu::PresentMode::AutoVsync
        {
            let sender = apply_animations_to
                .upgrade()
                .ok_or(anyhow!("Failed to dispatch apply animations (no receiver for ShellEvents anymore, application vanished)"))?;

            sender.send(ShellEvent::ApplyAnimations(renderer.window_id()))?;
        }

        // Apply scene changes after we retrieved the texture (because retrieving the texture may
        // take time, we want to wait until the last moment before pulling changes), even though the
        // time between retrieving the texture and final rendering takes longer. This reduces lag
        // noticeably (for example in the logs example while smooth rendering).
        renderer.apply_scene_changes()?;

        renderer.render_and_present(view_projection, texture);
        Ok(())
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn change_collector(&self) -> &Arc<ChangeCollector> {
        &self.change_collector
    }

    pub fn geometry(&self) -> &RenderGeometry {
        &self.geometry
    }

    pub fn view_projection(&mut self) -> Matrix {
        self.geometry.view_projection()
    }

    pub fn update_camera(&mut self, camera: Camera) -> Result<()> {
        self.geometry.set_camera(camera);
        self.redraw()
    }

    fn pacing(&self) -> RenderPacing {
        self.pacing
    }

    /// Changes the render pacing.
    fn update_render_pacing(&mut self, pacing: RenderPacing) -> Result<()> {
        if pacing == self.pacing {
            return Ok(());
        }

        info!("Changing renderer pacing to: {pacing:?}");

        let new_present_mode = match pacing {
            RenderPacing::Fast => wgpu::PresentMode::AutoNoVsync,
            RenderPacing::Smooth => wgpu::PresentMode::AutoVsync,
        };
        self.set_present_mode(new_present_mode)?;

        self.pacing = pacing;
        Ok(())
    }

    fn set_present_mode(&self, new_present_mode: wgpu::PresentMode) -> Result<()> {
        self.post_msg(RendererMessage::SetPresentMode(new_present_mode))
    }

    pub fn resize_redraw(&mut self, rrr: impl Into<ResizeRedrawRequest>) -> Result<()> {
        let rrr = rrr.into();

        if let Some(window) = rrr.window
            && window != self.window_id
        {
            return Ok(());
        }

        match rrr.mode {
            ResizeRedrawMode::Resize(wh) => {
                self.resize(wh)?;
                // We immediately issue a redraw after a resize. Usually, when a WindowEvent is used
                // to generate Resize / Redraw requests, another redraw will follow, but this takes
                // too long for smooth resizing.
                //
                // Robustness: Do this only in fast pacing mode?
                self.redraw()
            }
            ResizeRedrawMode::Redraw => {
                // Robustness: Do this only in fast pacing mode?
                self.redraw()
            }
            ResizeRedrawMode::None => Ok(()),
        }
    }

    pub fn resize(&mut self, surface_size: (u32, u32)) -> Result<()> {
        self.geometry.set_surface_size(surface_size);
        self.post_msg(RendererMessage::Resize(surface_size))
    }

    pub fn redraw(&mut self) -> Result<()> {
        let view_projection = self.geometry.view_projection();
        self.post_msg(RendererMessage::Redraw { view_projection })
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

/// Preliminary, not used yet.
#[derive(Debug)]
pub enum RenderMode {
    /// Nothing special, just render the changes if any.
    Default,
    /// Independent o the changes, just force a redraw.
    ForceRedraw,
    /// All animations for the current tick of the associated AnimationCoordinator were applied.
    /// This also forces a redraw to get a new presentation timestamp and in response a new
    /// ApplyAnimations.
    ///
    /// Architecture: It's perhaps wrong to make this dependent on a prior ApplyAnimations event. In
    /// Smooth mode we should perhaps render every frame completely autonomously in the render
    /// thread. A kind of pull mode.
    ///
    /// But then: When do we know that there are no new
    AnimationsApplied,
    /// Resize, no redraw is forced if there are no actual changes, because the windowing system is
    /// expected to issue a redraw event afterwards that ends up here with a ForceRedraw.
    Resize(u32, u32),
}

impl RenderTarget for AsyncWindowRenderer {
    fn render(&mut self, changes: SceneChanges, pacing: RenderPacing) -> Result<()> {
        // Update render pacing before a redraw:
        if pacing != self.pacing {
            info!("Changing render pacing to: {pacing:?}");
            self.update_render_pacing(pacing)?;
        }

        // Push the changes _directly_ to the renderer which picks it up in the next redraw. This
        // may asynchronously overtake the subsequent redraw / resize requests if a previous one is
        // currently on its way.
        //
        // Architecture: We could send this through the RendererMessage::Redraw, which may cause
        // other problems (increased latency and the need for combining changes if Redraws are
        // pending).
        //
        // Robustness: This should probably threaded through the redraw pipeline?
        if !changes.is_empty() {
            self.change_collector().push_many(changes);
            // Only in fast render pacing we issue a redraw request. Otherwise the renderer
            // takes care of it.
            if self.pacing() == RenderPacing::Fast {
                self.redraw()?;
            }
        }

        Ok(())
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
    Resize((u32, u32)),
    Redraw,
    #[default]
    None,
}

impl From<&event::WindowEvent> for ResizeRedrawRequest {
    fn from(window_event: &event::WindowEvent) -> Self {
        use event::WindowEvent;
        let mode = match window_event {
            WindowEvent::Resized(physical_size) => {
                ResizeRedrawMode::Resize((physical_size.width, physical_size.height))
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
