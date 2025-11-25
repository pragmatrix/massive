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
use massive_animation::AnimationCoordinator;
use massive_applications::RenderTarget;
use tokio::sync::mpsc::WeakUnboundedSender;
use winit::{event::WindowEvent, window::WindowId};

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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum RenderPacing {
    #[default]
    // Render as fast as possible to be able to represent input changes.
    Fast,
    // Render a smooth as possible so that animations are synced to the frame rate.
    Smooth,
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
    /// Does not return if there are no events.
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

        // Apply scene changes after we retrieved the texture (because retrieving the
        // texture may take time, we want to wait until the last moment before pulling
        // changes), even though the time between retrieving the texture and final rendering
        // takes longer.
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

    pub fn pacing(&self) -> RenderPacing {
        self.pacing
    }

    /// Changes the render pacing.
    pub fn update_render_pacing(&mut self, pacing: RenderPacing) -> Result<()> {
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
    type Event = ShellEvent;

    fn render(
        &mut self,
        changes: SceneChanges,
        animation_coordinator: &AnimationCoordinator,
        event: Option<Self::Event>,
    ) -> Result<()> {
        // End the current animation and see if animations are active.
        let animations_active = animation_coordinator.end_cycle();

        let mut redraw = false;

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
            redraw = true;
        }

        let window_id = self.window_id();
        let mut resize = None;
        let mut animations_applied = false;
        match event {
            Some(ShellEvent::WindowEvent(id, window_event)) if id == window_id => {
                match window_event {
                    WindowEvent::Resized(size) => {
                        resize = Some((size.width, size.height));
                        // Detail: We don't need to set a redraw here, there will _always_ be a
                        // redraw event afterwards after a resize event (winit 0.30, macos).
                    }
                    WindowEvent::RedrawRequested => {
                        redraw = true;
                    }
                    _ => {}
                }
            }
            // ADR: Decided to consider all ApplyAnimations, even the ones coming from other
            // windows. I.e. animations may be applied and visible on another window.
            Some(ShellEvent::ApplyAnimations(_)) => {
                // Even if nothing changed in apply animations, we have to redraw to get a new presentation timestamp.
                redraw = true;
                animations_applied = true;
            }
            _ => {}
        };

        let animations_before = self.pacing() == RenderPacing::Smooth;

        let new_render_pacing = match (animations_before, animations_active, animations_applied) {
            (false, true, _) => {
                // Changing from Fast to Smooth requires presentation timestamps to follow. So redraw.
                redraw = true;
                Some(RenderPacing::Smooth)
            }
            // Detail: Changing from Smooth to fast is only possible in response to
            // `ApplyAnimations`: Only then we know that animations are actually applied to the
            // scene and pushed to the renderer with this update.
            (true, false, true) => Some(RenderPacing::Fast),
            _ => None,
        };

        //
        // Sync with the renderer.
        //

        // Resize first
        if let Some(new_size) = resize {
            self.resize(new_size)?;
        }

        // Update render pacing before a redraw:
        // - from instant to smooth: We force a redraw _afterwards_ to get VSync based presentation
        //   timestamps and cause ApplyAnimations.
        // - from smooth to instant: Redraw only when something changed afterwards, but instantly
        //   without VSync.
        if let Some(new_render_pacing) = new_render_pacing {
            info!("Changing render pacing to: {new_render_pacing:?}");
            self.update_render_pacing(new_render_pacing)?;
        }

        // Only in fast render pacing we issue a redraw request. Otherwise the renderer
        // automatically takes care of it.
        if redraw && self.pacing() == RenderPacing::Fast {
            self.redraw()?;
        }

        Ok(())
    }
}
