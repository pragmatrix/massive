use std::{
    mem,
    ops::Deref,
    result,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use cosmic_text::FontSystem;
use log::error;
use tokio::sync::oneshot;
use wgpu::rwh;
use winit::{
    dpi::PhysicalSize,
    event_loop::EventLoopProxy,
    window::{Window, WindowId},
};

use crate::{AsyncWindowRenderer, shell::ShellRequest, window_renderer::WindowRenderer};
use massive_geometry::Camera;

#[derive(Clone)]
pub struct ShellWindow {
    /// We need to make Window "sharable", because the Renderer needs to lock it, so that it does
    /// not close with a renderer running.
    shared: Arc<ShellWindowShared>,
}

// Architecture: May expose all of the Window?
impl Deref for ShellWindow {
    type Target = ShellWindowShared;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

impl ShellWindow {
    pub(crate) fn new(window: Window, event_loop_proxy: EventLoopProxy<ShellRequest>) -> Self {
        Self {
            shared: ShellWindowShared {
                window: Some(window),
                event_loop_proxy,
            }
            .into(),
        }
    }

    pub fn id(&self) -> WindowId {
        self.shared.id()
    }

    pub fn set_title(&self, title: &str) {
        self.shared.window().set_title(title);
    }

    // DI: Use SizeI to represent initial_size.
    pub async fn new_renderer(
        &self,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // Feature: Use a rect here to place the renderer on the window.
        // (But what about resizes then?)
        // Ergonomics: Don't use winit types.
        initial_size: impl Into<PhysicalSize<u32>>,
    ) -> Result<AsyncWindowRenderer> {
        let instance_and_surface = self
            .new_instance_and_surface(
                wgpu::InstanceDescriptor::default(),
                // Use this for testing webgl:
                // InstanceDescriptor {
                //     backends: wgpu::Backends::GL,
                //     ..InstanceDescriptor::default()
                // },
                self.shared.clone(),
            )
            .await;
        // On wasm, attempt to fall back to webgl
        #[cfg(target_arch = "wasm32")]
        let instance_and_surface = match instance_and_surface {
            Ok(_) => instance_and_surface,
            Err(_) => self.new_instance_and_surface(
                InstanceDescriptor {
                    backends: wgpu::Backends::GL,
                    ..InstanceDescriptor::default()
                },
                self.window.clone(),
            ),
        }
        .await;
        let (instance, surface) = instance_and_surface?;

        // DI: If we can access the ShellWindow, we don't need a clone of font_system or
        // event_loop_proxy here.
        let window_renderer = WindowRenderer::new(
            self.shared.clone(),
            instance,
            surface,
            font_system,
            camera,
            initial_size.into(),
        )
        .await?;

        let async_window_renderer = AsyncWindowRenderer::new(window_renderer);
        Ok(async_window_renderer)
    }

    /// Helper to create instance and surface.
    ///
    /// A function here, because we may try multiple times.
    async fn new_instance_and_surface(
        &self,
        instance_descriptor: wgpu::InstanceDescriptor,
        window: Arc<ShellWindowShared>,
    ) -> Result<(wgpu::Instance, wgpu::Surface<'static>)> {
        let instance = wgpu::Instance::new(&instance_descriptor);

        let (on_created, when_created) = oneshot::channel();

        self.shared
            .event_loop_proxy
            .send_event(ShellRequest::CreateSurface {
                instance: instance.clone(),
                window,
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;
        let surface = when_created.await.expect("oneshot receive");
        Ok((instance, surface?))
    }
}

#[derive(Debug)]
pub struct ShellWindowShared {
    // ADR: Option, because we have to send it back to the event loop for closing.
    window: Option<Window>,
    // For creating surfaces, we need to communicate with the Shell.
    event_loop_proxy: EventLoopProxy<ShellRequest>,
}

impl Drop for ShellWindowShared {
    fn drop(&mut self) {
        let window = self.window.take().unwrap();
        if let Err(e) = self
            .event_loop_proxy
            .send_event(ShellRequest::DestroyWindow { window })
        {
            error!("Failed to send back Window to the event loop (Event loop closed)");
            // Dropping it here would most likely block this thread indefinitely, so we forget the
            // window, which is in the ShellRequest returned in the Error.
            mem::forget(e)
        }
    }
}

impl ShellWindowShared {
    pub fn scale_factor(&self) -> f64 {
        self.window().scale_factor()
    }

    pub fn id(&self) -> WindowId {
        self.window().id()
    }

    pub fn request_redraw(&self) {
        self.window().request_redraw()
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.window().inner_size()
    }

    fn window(&self) -> &Window {
        self.window.as_ref().unwrap()
    }
}

// Forward wgpu requirements to the window. This is so that we can create a SurfaceTarget.
//
// We can't pass the Arc<Window> to the Surface target, because then we would not know from where
// (its last instance) it gets destroyed and could not guarantee that this is done on the event
// loop.

impl rwh::HasDisplayHandle for ShellWindowShared {
    fn display_handle(&self) -> result::Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        self.window().display_handle()
    }
}

impl rwh::HasWindowHandle for ShellWindowShared {
    fn window_handle(&self) -> result::Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        self.window().window_handle()
    }
}
