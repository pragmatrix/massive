use std::{
    mem,
    ops::Deref,
    result,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Result};
use cosmic_text::FontSystem;
use log::{error, info};
use tokio::sync::oneshot;
use wgpu::{rwh, Instance, InstanceDescriptor, Surface, SurfaceTarget};
use winit::{
    dpi::PhysicalSize,
    event_loop::EventLoopProxy,
    window::{Window, WindowId},
};

use crate::{shell::ShellRequest, window_renderer::WindowRenderer, AsyncWindowRenderer};
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

    // DI: Use SizeI to represent initial_size.
    pub async fn new_renderer(
        &self,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // Use a rect here to place the renderer on the window.
        // (But what about resizes then?)
        initial_size: PhysicalSize<u32>,
    ) -> Result<AsyncWindowRenderer> {
        let instance_and_surface = self
            .new_instance_and_surface(
                InstanceDescriptor::default(),
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
            initial_size,
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
        instance_descriptor: InstanceDescriptor,
        surface_target: Arc<ShellWindowShared>,
    ) -> Result<(Instance, Surface<'static>)> {
        let instance = wgpu::Instance::new(&instance_descriptor);

        let surface_target: SurfaceTarget<'static> = surface_target.into();
        info!(
            "Creating surface on a {} target",
            match surface_target {
                SurfaceTarget::Window(_) => "Window",
                #[cfg(target_arch = "wasm32")]
                SurfaceTarget::Canvas(_) => "Canvas",
                #[cfg(target_arch = "wasm32")]
                SurfaceTarget::OffscreenCanvas(_) => "OffscreenCanvas",
                _ => "(Undefined SurfaceTarget, Internal Error)",
            }
        );

        let (on_created, when_created) = oneshot::channel();

        self.shared
            .event_loop_proxy
            .send_event(ShellRequest::CreateSurface {
                instance: instance.clone(),
                target: surface_target,
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;
        let surface = when_created.await.expect("oneshot receive");
        Ok((instance, surface?))
    }
}

pub struct ShellWindowShared {
    // Option, because we have to send it back to the event loop for closing.
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
            // Dropping it here would most likely block indefinitely this thread, so we forget the
            // window.
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
// We can't pass the Arc<Window> to the Surface target, becase then we would not know from where
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
