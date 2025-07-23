use std::{
    mem,
    sync::mpsc::{self, channel, Sender},
    thread::{self, JoinHandle},
};
use winit::{
    event::WindowEvent,
    window::{Window, WindowId},
};

use anyhow::{anyhow, Result};
use log::error;

use crate::window_renderer::WindowRenderer;
use massive_geometry::Camera;

enum RendererMessage {
    WindowEvent(WindowEvent),
    UpdateCamera(Camera),
}

pub struct AsyncWindowRenderer {
    id: WindowId,
    sender: Sender<RendererMessage>,
    thread_handle: Option<JoinHandle<()>>,
}

impl AsyncWindowRenderer {
    pub fn new(mut window_renderer: WindowRenderer) -> Self {
        let id = window_renderer.id();
        let (sender, receiver) = channel();

        let thread_handle = thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                match message {
                    RendererMessage::WindowEvent(event) => {
                        if let Err(e) = window_renderer.handle_window_event(&event) {
                            error!("Error handling window event: {e:?}");
                        }
                    }
                    RendererMessage::UpdateCamera(camera) => {
                        window_renderer.update_camera(camera);
                    }
                }
            }
            // The loop will exit when the channel is closed (when sender is dropped)
        });

        Self {
            id,
            sender,
            thread_handle: Some(thread_handle),
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn should_handle_window_event(event: &WindowEvent) -> bool {
        matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::RedrawRequested
        )
    }

    pub fn handle_window_event(&self, event: &WindowEvent) -> Result<()> {
        if !Self::should_handle_window_event(event) {
            // Not handled, just ignore it.
            return Ok(());
        }

        self.sender
            .send(RendererMessage::WindowEvent(event.clone()))
            .map_err(|e| anyhow!("Failed to send window event: {e:?}"))?;
        Ok(())
    }

    pub fn update_camera(&self, camera: Camera) -> Result<()> {
        self.sender
            .send(RendererMessage::UpdateCamera(camera))
            .map_err(|e| anyhow!("Failed to send camera update: {e:?}"))?;
        Ok(())
    }
}

impl Drop for AsyncWindowRenderer {
    fn drop(&mut self) {
        // Explicitly drop the sender first to close the channel
        // This will cause the receiving thread to exit
        mem::drop(mem::replace(&mut self.sender, mpsc::channel().0));

        // Then join the thread to ensure clean shutdown
        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                error!("Error joining AsyncWindowRenderer thread: {e:?}");
            }
        }
    }
}
