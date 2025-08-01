use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cosmic_text::FontSystem;
use log::info;
use tokio::sync::oneshot;
use wgpu::{Instance, InstanceDescriptor, Surface, SurfaceTarget};
use winit::{
    dpi::PhysicalSize,
    event_loop::EventLoopProxy,
    window::{Window, WindowId},
};

use crate::{shell::ShellRequest, window_renderer::WindowRenderer, AsyncWindowRenderer};
use massive_geometry::Camera;

pub struct ShellWindow {
    /// `Arc` because this is shared with the renderer because it needs to invoke request_redraw(), too.
    pub(crate) window: Arc<Window>,
    // For creating surfaces, we need to communicate with the Shell.
    pub(crate) event_loop_proxy: EventLoopProxy<ShellRequest>,
}

impl ShellWindow {
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
                self.window.clone(),
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
            self.window.clone(),
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
        surface_target: Arc<Window>,
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

        self.event_loop_proxy
            .send_event(ShellRequest::CreateSurface {
                instance: instance.clone(),
                target: surface_target,
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;
        let surface = when_created.await.expect("oneshot receive");
        Ok((instance, surface?))
    }

    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw()
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }
}
