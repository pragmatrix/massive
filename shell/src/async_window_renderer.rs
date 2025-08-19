use std::{
    collections::HashMap,
    mem,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use log::error;
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel,
};
use winit::window::WindowId;

use crate::window_renderer::WindowRenderer;
use massive_geometry::{Camera, Color};
use massive_scene::ChangeCollector;

#[derive(Debug)]
pub struct AsyncWindowRenderer {
    window_id: WindowId,
    // For pushing changes directly to the renderer.
    change_collector: Arc<ChangeCollector>,
    msg_sender: Sender<RendererMessage>,
    presentation_receiver: UnboundedReceiver<Instant>,
    thread_handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub enum RendererMessage {
    Resize((u32, u32)),
    Redraw,
    // This looks alien here.
    UpdateCamera(Camera),
    SetPresentMode(wgpu::PresentMode),
    SetBackgroundColor(Option<Color>),
    // Protocol: When adding a new RenderMessage, consider filter_latest_messages().
}

impl AsyncWindowRenderer {
    pub fn new(mut window_renderer: WindowRenderer) -> Self {
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

                // Process only the latest message of each type
                let latest_messages = Self::filter_latest_messages(messages);

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
        }
    }

    /// Filters messages to keep only the latest occurrence of each message type,
    /// preserving the original order of the remaining messages.
    fn filter_latest_messages(messages: Vec<RendererMessage>) -> Vec<RendererMessage> {
        // Find the latest index for each message type
        let mut latest_index_by_type: HashMap<mem::Discriminant<RendererMessage>, usize> =
            HashMap::new();

        for (index, message) in messages.iter().enumerate() {
            let discriminant = mem::discriminant(message);
            latest_index_by_type.insert(discriminant, index);
        }

        // Collect messages at those indices, preserving original order
        let mut result = Vec::new();
        for (index, message) in messages.into_iter().enumerate() {
            let discriminant = mem::discriminant(&message);
            if latest_index_by_type.get(&discriminant) == Some(&index) {
                result.push(message);
            }
        }

        result
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
            RendererMessage::UpdateCamera(camera) => {
                renderer.update_camera(camera);
            }
            RendererMessage::Redraw => {
                let texture = renderer.apply_scene_changes_and_prepare_presentation()?;
                presentation_timestamps.send(Instant::now())?;
                renderer.render_and_present(texture);
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

    pub fn post_msg(&self, message: RendererMessage) -> Result<()> {
        self.msg_sender
            .send(message)
            .context("Sending renderer message")?;
        Ok(())
    }

    pub fn update_camera(&self, camera: Camera) -> Result<()> {
        self.msg_sender
            .send(RendererMessage::UpdateCamera(camera))
            .map_err(|e| anyhow!("Failed to send camera update: {e:?}"))?;
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
