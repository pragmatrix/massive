use std::{
    mem,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result, anyhow};
use log::{error, info};
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
        mut window_renderer: WindowRenderer,
        geometry: RenderGeometry,
        shell_events: Option<WeakUnboundedSender<ShellEvent>>,
    ) -> Self {
        let id = window_renderer.window_id();
        let change_collector = window_renderer.change_collector().clone();

        let (msg_sender, msg_receiver) = mpsc::channel();

        let thread_handle = thread::spawn(move || {
            while let Ok(first_message) = msg_receiver.recv() {
                // Collect all pending messages without blocking
                let mut messages = vec![first_message];
                while let Ok(message) = msg_receiver.try_recv() {
                    messages.push(message);
                }

                // Process only the latest message of each variant
                let latest_messages = keep_last_per_variant(messages, |_| true);

                for message in latest_messages {
                    if let Err(e) = Self::dispatch(&mut window_renderer, &shell_events, message) {
                        // Robustness: What to do here, we need to inform the application, don't we?
                        log::error!("Render error: {e:?}");
                    }
                }
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

    fn dispatch(
        renderer: &mut WindowRenderer,
        apply_animations_to: &Option<WeakUnboundedSender<ShellEvent>>,
        message: RendererMessage,
    ) -> Result<()> {
        match message {
            // Optimization: If we resize and change present mode the same time, we would only need to reconfigure
            // the surface once. Renderer might even reconfigure lazily.
            RendererMessage::Resize(new_size) => {
                renderer.resize(new_size);
            }
            RendererMessage::SetPresentMode(present_mode) => {
                // Detail: Switching from NoVSync to VSync takes ~200 microseconds,
                // from VSync to NoVSync around ~2.7 milliseconds (measured in the logs example --release).
                renderer.set_present_mode(present_mode);
            }
            RendererMessage::Redraw { view_projection } => {
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
            }
            RendererMessage::SetBackgroundColor(color) => {
                renderer.set_background_color(color);
            }
        }
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

impl RenderTarget for AsyncWindowRenderer {
    type Event = ShellEvent;

    fn render(
        &mut self,
        changes: SceneChanges,
        animation_coordinator: &AnimationCoordinator,
        event: Option<Self::Event>,
    ) -> Result<()> {
        // End the current animation and see if animations are active.
        //
        // ADR: This was moved from the render_to() function to here, because we want may want to push
        // scene changes multiple times per frame (for example in the desktop renderer).
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
                    WindowEvent::RedrawRequested => {
                        redraw = true;
                    }
                    WindowEvent::Resized(size) => {
                        resize = Some((size.width, size.height));
                        // Robustness: Is this needed. Doesn't the system always send a redraw
                        // anyway after each resize?
                        redraw = true
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

        // Resize first and follow up with a complete redraw.

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

        if redraw {
            self.redraw()?;
        }

        Ok(())
    }
}
