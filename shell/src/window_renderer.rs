use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use cosmic_text::FontSystem;
use log::info;
use wgpu::{PresentMode, Surface, TextureFormat};
use winit::window::WindowId;

use crate::shell_window::ShellWindowShared;
use massive_geometry::{Color, Matrix4};
use massive_renderer::Renderer;
use massive_scene::ChangeCollector;

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;
const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::PUSH_CONSTANTS;

pub struct WindowRenderer {
    window: Arc<ShellWindowShared>,
    font_system: Arc<Mutex<FontSystem>>,
    change_collector: Arc<ChangeCollector>,
    renderer: Renderer,
}

impl WindowRenderer {
    pub async fn new(
        window: Arc<ShellWindowShared>,
        instance: wgpu::Instance,
        surface: Surface<'static>,
        font_system: Arc<Mutex<FontSystem>>,
        // TODO: use a rect here to be able to position the surface on the window!
        initial_surface_size: (u32, u32),
    ) -> Result<WindowRenderer> {
        info!("Getting adapter");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                // Be sure the adapter can present the surface.
                compatible_surface: Some(&surface),
                // software fallback?
                force_fallback_adapter: false,
            })
            .await
            .expect("Adapter not found");

        if !adapter.features().contains(REQUIRED_FEATURES) {
            bail!("GPU must support {:?}", REQUIRED_FEATURES);
        }

        info!("GPU backend: {:?}", adapter.get_info().backend,);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: REQUIRED_FEATURES,
                // May be wrong, see: <https://github.com/gfx-rs/wgpu/blob/1144b065c4784d769d59da2f58f5aa13212627b0/examples/src/hello_triangle/mod.rs#L33-L34>
                required_limits: adapter.limits(),
                label: None,
                memory_hints: Default::default(),
                trace: Default::default(),
            })
            .await
            .context("Requesting device")?;

        info!(
            "Max texture dimension: {}",
            device.limits().max_texture_dimension_2d
        );

        let surface_caps = surface.get_capabilities(&adapter);

        // Don't use srgb now, colors are specified in linear rgb space.
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);

        info!("Surface format: {surface_format:?}");

        info!("Available present modes: {:?}", surface_caps.present_modes);

        let alpha_mode = surface_caps.alpha_modes[0];

        info!("Selecting alpha mode: {alpha_mode:?}, initial size: {initial_surface_size:?}",);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_surface_size.0,
            height: initial_surface_size.1,
            // 20250721: Since the time we are rendering asynchronously, not bound to the main
            // thread, VSync seems to be fast enough on MacOS and also fixes the "wobbly" resizing.
            //
            // 20250724: This is not true, on my MacBook Pro with the mouse, this is considerably
            // slower. So we perhaps have to switch between interactive mode (Immediate, and VSync
            // for animations). Also the "wobbly" resizing appears again with VSync.
            present_mode: PresentMode::AutoNoVsync,
            // Robustness: Select explicitly
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);
        let renderer = Renderer::new(device, queue, surface, surface_config, font_system.clone());

        let window_renderer = WindowRenderer {
            window: window.clone(),
            font_system,
            change_collector: Arc::new(ChangeCollector::default()),
            renderer,
        };

        Ok(window_renderer)
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn font_system(&self) -> &Arc<Mutex<FontSystem>> {
        &self.font_system
    }

    pub fn change_collector(&self) -> &Arc<ChangeCollector> {
        &self.change_collector
    }

    /// The format chosen for the swapchain.
    pub fn surface_format(&self) -> TextureFormat {
        self.renderer.surface_config.format
    }

    // Surface size may not match the Window's size, for example if the window's size is 0,0.
    pub fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }

    /// Sets the background color for the next redraw.
    ///
    /// Does not request a redraw.
    pub fn set_background_color(&mut self, color: Option<Color>) {
        self.renderer.background_color = color;
    }
}

impl WindowRenderer {
    pub(crate) fn resize(&mut self, new_size: (u32, u32)) {
        self.renderer.resize_surface(new_size)
    }

    pub(crate) fn set_present_mode(&mut self, present_mode: PresentMode) {
        self.renderer.set_present_mode(present_mode);
    }

    /// Apply all changes to the renderer and prepare the presentation.
    ///
    /// This is separate from render_and_present.
    pub(crate) fn apply_scene_changes_and_prepare_presentation(
        &mut self,
    ) -> Result<wgpu::SurfaceTexture> {
        let changes = self.change_collector.take_all();

        self.renderer.apply_changes(changes)?;

        self.renderer.prepare()?;

        // Important: This blocks in VSync modes until the previous frame is presented.
        // Robustness: Learn about how to recover from specific `SurfaceError`s errors here
        // `get_current_texture()` tries once.
        let texture = self
            .renderer
            .get_current_texture()
            .context("get_current_texture")?;

        Ok(texture)
    }

    pub(crate) fn render_and_present(
        &mut self,
        view_projection_matrix: Matrix4,
        texture: wgpu::SurfaceTexture,
    ) {
        self.renderer
            .render_and_present(&view_projection_matrix, texture)
    }
}
