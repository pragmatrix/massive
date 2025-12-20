use std::sync::Arc;

use anyhow::Result;

use log::debug;
use massive_geometry::{Camera, Color, SizePx};
use massive_renderer::{FontManager, RenderDevice, RenderGeometry, RendererBuilder};

use crate::{AsyncWindowRenderer, WindowRenderer, shell_window::ShellWindowShared};

#[derive(Debug)]
pub struct WindowRendererBuilder {
    window: Arc<ShellWindowShared>,
    /// Default is window's inner size.
    initial_size: Option<SizePx>,

    camera: Option<Camera>,
    background_color: Option<Color>,
    shapes: bool,
    text: Option<FontManager>,
    measurements: bool,
}

impl WindowRendererBuilder {
    // Async because requesting an adapter and device / queue is async in wgpu.
    // Architecture: May create a function that creates this in one go.
    pub(crate) fn new(window: Arc<ShellWindowShared>) -> Self {
        Self {
            window,
            initial_size: None,
            camera: None,
            background_color: None,
            shapes: false,
            text: None,
            measurements: false,
        }
    }

    pub fn with_size(mut self, size: impl Into<SizePx>) -> Self {
        self.initial_size = Some(size.into());
        self
    }

    /// Sets the background color the renderer begins with.
    pub fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Overrides the default camera.
    ///
    /// By default the camera is set so that the scene is rendered at z 0 at a depth so that one
    /// unit (one pixel) corresponds to one physical pixel on the screen.
    pub fn with_camera(mut self, camera: Camera) -> Self {
        self.camera = Some(camera);
        self
    }

    /// Adds support for shape rendering.
    ///
    /// By default, no shape rendering is available.
    pub fn with_shapes(mut self) -> Self {
        self.shapes = true;
        self
    }

    /// Enables text / font rendering support.
    ///
    /// By default, no font / GlyphRun support is available.
    pub fn with_text(mut self, fonts: FontManager) -> Self {
        self.text = Some(fonts);
        self
    }

    /// Measure performance.
    ///
    /// Default is off.
    pub fn with_measurements(mut self) -> Self {
        self.measurements = true;
        self
    }

    pub async fn build(self) -> Result<AsyncWindowRenderer> {
        let instance_and_surface = self
            .window
            .new_instance_and_surface(
                wgpu::InstanceDescriptor::default(),
                // Use this for testing WebGL:
                // InstanceDescriptor {
                //     backends: wgpu::Backends::GL,
                //     ..InstanceDescriptor::default()
                // },
                self.window.clone(),
            )
            .await;
        // On Wasm, attempt to fall back to WebGL
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

        let device = RenderDevice::for_surface(instance, &surface).await?;

        let initial_size = self
            .initial_size
            .unwrap_or_else(|| self.window.inner_size());

        // Robustness: This is here to see if the initial size got resolved properly from the
        // Window's inner size.
        debug!("Renderer initial size: {initial_size:?}");

        let renderer = {
            let mut builder = RendererBuilder::new(device, surface, initial_size);

            if let Some(color) = self.background_color {
                builder = builder.with_background_color(color);
            }
            if self.shapes {
                builder = builder.with_shapes();
            }
            if let Some(fonts) = self.text {
                builder = builder.with_text(fonts);
            }
            if self.measurements {
                builder = builder.with_measurements();
            }

            builder.build()
        };

        let event_sender = self.window.event_sender.clone();
        let window_renderer = WindowRenderer::new(self.window, renderer);

        let camera = self
            .camera
            .unwrap_or_else(|| Camera::pixel_aligned(Camera::DEFAULT_FOVY));

        let render_geometry = RenderGeometry::new(initial_size, camera);

        Ok(AsyncWindowRenderer::new(
            window_renderer,
            render_geometry,
            event_sender,
        ))
    }
}
