use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use log::info;
use wgpu::{PresentMode, Surface, TextureFormat};
use winit::{dpi::PhysicalSize, event::WindowEvent, window::WindowId};

use crate::shell_window::ShellWindowShared;
use cosmic_text::FontSystem;
use massive_geometry::{Camera, Color, Matrix4, scalar};
use massive_renderer::Renderer;
use massive_scene::ChangeCollector;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);
const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;
const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::PUSH_CONSTANTS;

pub struct WindowRenderer {
    window: Arc<ShellWindowShared>,
    font_system: Arc<Mutex<FontSystem>>,
    camera: Camera,
    change_collector: Arc<ChangeCollector>,
    renderer: Renderer,
}

impl WindowRenderer {
    pub async fn new(
        window: Arc<ShellWindowShared>,
        instance: wgpu::Instance,
        surface: Surface<'static>,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // TODO: use a rect here to be able to position the renderer!
        initial_size: PhysicalSize<u32>,
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

        info!("Selecting alpha mode: {alpha_mode:?}, initial size: {initial_size:?}",);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_size.width,
            height: initial_size.height,
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
            camera,
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

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    pub fn pixel_matrix(&self) -> Matrix4 {
        self.renderer.pixel_matrix()
    }

    // Surface size may not match the Window's size, for example if the window's size is 0,0.
    #[allow(unused)]
    pub fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }

    /// Sets the background color for the next redraw.
    ///
    /// Does not request a redraw.
    pub fn set_background_color(&mut self, color: Option<Color>) {
        self.renderer.background_color = color;
    }

    // DI: If the renderer does culling, we need to move the camera (or at least the view matrix) into the renderer, and
    // perhaps schedule updates using the director.
    pub fn update_camera(&mut self, camera: Camera) {
        self.camera = camera;
        // Robustness: We probably should draw this directly in the end of the next cycle.
        // This way we would not need to hold a Window handle here anymore.
        self.window.request_redraw();
    }

    pub fn handle_window_event(&mut self, window_event: &WindowEvent) -> Result<()> {
        match window_event {
            WindowEvent::Resized(physical_size) => {
                info!("{window_event:?}");
                // Robustness: Put this into a spawn_blocking inside when run in an async runtime.
                // Last time measured: This takes around 40 to 60 microseconds.
                self.resize((physical_size.width, physical_size.height));
                // 20250721: Disabled, because redraw is happening automatically, and otherwise
                // will slow things down.
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let new_inner_size = self.window.inner_size();
                self.resize((new_inner_size.width, new_inner_size.height));
            }
            WindowEvent::RedrawRequested => {
                // This may block when VSync is enabled and when the previous frame
                // wasn't rendered yet.
                self.redraw()?;
            }
            _ => {}
        }

        Ok(())
    }
}

impl WindowRenderer {
    pub(crate) fn resize(&mut self, new_size: (u32, u32)) {
        self.renderer.resize_surface(new_size)
    }

    pub(crate) fn set_present_mode(&mut self, present_mode: PresentMode) {
        self.renderer.set_present_mode(present_mode);
    }

    pub(crate) fn redraw(&mut self) -> Result<()> {
        let texture = self.apply_scene_changes_and_prepare_presentation()?;
        self.render_and_present(texture);
        Ok(())
    }

    /// Apply all changes to the renderer and prepare the presentation.
    ///
    /// This is separate from render_and_present.
    pub(crate) fn apply_scene_changes_and_prepare_presentation(
        &mut self,
    ) -> Result<wgpu::SurfaceTexture> {
        let changes = self.change_collector.take_all();

        self.renderer.apply_changes(changes)?;

        self.renderer.prepare();

        // Important: This blocks in VSync modes until the previous frame is presented.
        // Robustness: Learn about how to recover from specific `SurfaceError`s errors here
        // `get_current_texture()` tries once.
        let texture = self
            .renderer
            .get_current_texture()
            .context("get_current_texture")?;

        Ok(texture)
    }

    pub(crate) fn render_and_present(&mut self, texture: wgpu::SurfaceTexture) {
        let view_projection_matrix = self.view_projection_matrix();
        self.renderer
            .render_and_present(&view_projection_matrix, texture)
    }

    fn view_projection_matrix(&self) -> Matrix4 {
        let surface_size = self.renderer.surface_size();
        self.camera.view_projection_matrix(Z_RANGE, surface_size)
    }
}
