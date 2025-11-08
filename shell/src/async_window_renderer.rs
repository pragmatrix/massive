use std::{
    mem,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use log::{error, info};
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel,
};
use winit::window::WindowId;

use crate::{message_filter::keep_last_per_variant, window_renderer::WindowRenderer};
use massive_geometry::{Camera, Color};
use massive_renderer::RenderGeometry;
use massive_scene::{ChangeCollector, Matrix};

#[derive(Debug)]
pub struct AsyncWindowRenderer {
    window_id: WindowId,
    // For pushing changes directly to the renderer.
    change_collector: Arc<ChangeCollector>,
    msg_sender: Sender<RendererMessage>,
    presentation_receiver: UnboundedReceiver<Instant>,
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
    pub fn new(mut window_renderer: WindowRenderer, geometry: RenderGeometry) -> Self {
        let id = window_renderer.window_id();
        let change_collector = window_renderer.change_collector().clone();

        let (msg_sender, msg_receiver) = mpsc::channel();
        // Robustness: Limiting this channel's size, we could detect rendering lag.
        let (presentation_sender, presentation_receiver) = unbounded_channel();

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
                    if let Err(e) =
                        Self::dispatch(&mut window_renderer, &presentation_sender, message)
                    {
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
            presentation_receiver,
            thread_handle: Some(thread_handle),
            geometry,
            pacing: Default::default(),
        }
    }

    fn dispatch(
        renderer: &mut WindowRenderer,
        presentation_timestamps: &UnboundedSender<Instant>,
        message: RendererMessage,
    ) -> Result<()> {
        match message {
            // Optimization: If we resize and change present mode the same time, we would only need to reconfigure
            // the surface once. Renderer might even reconfigure lazily.
            RendererMessage::Resize(new_size) => {
                renderer.resize(new_size);
            }
            RendererMessage::SetPresentMode(present_mode) => {
                renderer.set_present_mode(present_mode);
            }
            RendererMessage::Redraw { view_projection } => {
                let texture = renderer.apply_scene_changes_and_prepare_presentation()?;
                presentation_timestamps.send(Instant::now())?;
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

    /// Synchronizes the render pacing suggested by the current state of the tickery with the renderer.
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

    /// Wait for the most recent presentation.
    ///
    /// If multiple presentation Instants are available, the most recent one is returned.
    ///
    /// This is cancel safe.
    pub async fn wait_for_most_recent_presentation(&mut self) -> Result<Instant> {
        let most_recent = self.presentation_receiver.recv().await;
        let Some(mut most_recent) = most_recent else {
            bail!("Presentation sender vanished (thread got terminated?)");
        };

        // Get the most recent one available.
        loop {
            match self.presentation_receiver.try_recv() {
                Ok(instant) => most_recent = instant,
                Err(TryRecvError::Empty) => return Ok(most_recent),
                Err(TryRecvError::Disconnected) => {
                    bail!("Presentation sender vanished (thread got terminated?)");
                }
            }
        }
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
