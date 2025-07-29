use std::{
    mem,
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{anyhow, bail, Context, Result};
use log::error;
use tokio::sync::mpsc::{
    error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender,
};
use wgpu::PresentMode;
use winit::{event::WindowEvent, window::WindowId};

use crate::window_renderer::WindowRenderer;
use massive_geometry::Camera;

#[derive(Debug)]
pub struct AsyncWindowRenderer {
    id: WindowId,
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
    SetPresentMode(PresentMode),
}

impl AsyncWindowRenderer {
    pub fn new(mut window_renderer: WindowRenderer) -> Self {
        let id = window_renderer.id();
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
            id,
            msg_sender,
            presentation_receiver,
            thread_handle: Some(thread_handle),
        }
    }

    /// Filters messages to keep only the latest occurrence of each message type,
    /// preserving the original order of the remaining messages.
    fn filter_latest_messages(messages: Vec<RendererMessage>) -> Vec<RendererMessage> {
        use std::collections::HashMap;
        use std::mem;

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
        }
        Ok(())
    }

    pub fn window_id(&self) -> WindowId {
        self.id
    }

    // Architecture: Move this out of the impl, it is second nature now.
    pub fn should_handle_window_event(event: &WindowEvent) -> bool {
        // 202507: According to ChatGPT winit since version 0.29 may send additional Resize events when
        // ScaleFactorChanged is sent, so we don't handle ScaleFactorChanged here anymore.
        matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::RedrawRequested
        )
    }

    // Architecture: Move this out of the impl, it is second nature now.
    pub fn handle_window_event(&self, event: &WindowEvent) -> Result<()> {
        let event = match event {
            WindowEvent::Resized(new_size) => {
                RendererMessage::Resize((new_size.width, new_size.height))
            }
            WindowEvent::RedrawRequested => RendererMessage::Redraw,
            _ => {
                // Not something we are interested in
                return Ok(());
            }
        };

        self.post_msg(event)
    }

    pub fn post_msg(&self, event: RendererMessage) -> Result<()> {
        self.msg_sender
            .send(event)
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
        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                error!("Error joining AsyncWindowRenderer thread: {e:?}");
            }
        }
    }
}
