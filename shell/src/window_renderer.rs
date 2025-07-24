use std::{
    mem,
    ops::DerefMut,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use log::{error, info};
use massive_scene::SceneChange;
use wgpu::{PresentMode, Surface, TextureFormat};
use winit::{
    dpi::PhysicalSize,
    event::WindowEvent,
    window::{Window, WindowId},
};

use cosmic_text::FontSystem;
use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::Renderer;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);
const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

pub struct WindowRenderer {
    window: Arc<Window>,
    font_system: Arc<Mutex<FontSystem>>,
    camera: Camera,
    scene_changes: Arc<Mutex<Vec<SceneChange>>>,
    renderer: Renderer,
}

impl WindowRenderer {
    pub async fn new(
        window: Arc<Window>,
        instance: wgpu::Instance,
        surface: Surface<'static>,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // TODO: use a rect here to be able to position the renderer!
        initial_size: PhysicalSize<u32>,
    ) -> Result<(WindowRenderer, massive_scene::Director)> {
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

        info!("Effective WebGPU backend: {:?}", adapter.get_info().backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                // May be wrong, see: <https://github.com/gfx-rs/wgpu/blob/1144b065c4784d769d59da2f58f5aa13212627b0/examples/src/hello_triangle/mod.rs#L33-L34>
                required_limits: adapter.limits(),
                label: None,
                memory_hints: Default::default(),
                trace: Default::default(),
            })
            .await
            .context("Requesting device")?;

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
            // for animations).
            present_mode: PresentMode::AutoNoVsync,
            // Robustness: Select explicitly
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);
        let renderer = Renderer::new(device, queue, surface, surface_config);

        let scene_changes = Arc::new(Mutex::new(Vec::new()));

        let window_renderer = WindowRenderer {
            window: window.clone(),
            font_system,
            camera,
            scene_changes: scene_changes.clone(),
            renderer,
        };

        let window = window.clone();

        let director = massive_scene::Director::new(move |changes| {
            // Since we are the only one pushing to the renderer, we can invoke a request redraw
            // only once as soon `scene_changes` switch from empty to non-empty.
            let request_redraw = {
                let mut scene_changes = scene_changes.lock().unwrap();
                let was_empty = scene_changes.is_empty();
                scene_changes.extend(changes);
                was_empty && !scene_changes.is_empty()
            };
            if request_redraw {
                window.request_redraw();
            }
            Ok(())
        });

        Ok((window_renderer, director))
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn font_system(&self) -> &Arc<Mutex<FontSystem>> {
        &self.font_system
    }

    // DI: If the renderer does culling, we need to move the camera (or at least the view matrix) into the renderer, and
    // perhaps schedule updates using the director.
    pub fn update_camera(&mut self, camera: Camera) {
        self.camera = camera;
        self.window.request_redraw();
    }

    pub fn handle_window_event(&mut self, window_event: &WindowEvent) -> Result<()> {
        match window_event {
            WindowEvent::Resized(physical_size) => {
                info!("{window_event:?}");
                // Robustness: Put this into a spawn_blocking inside when run in an async runtime.
                // Last time measured: This takes around 40 to 60 microseconds.
                self.renderer
                    .resize_surface((physical_size.width, physical_size.height));
                // 20250721: Disabled, because redraw is happening automatically, and otherwise
                // will slow things down.
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let new_inner_size = self.window.inner_size();
                self.renderer
                    .resize_surface((new_inner_size.width, new_inner_size.height));
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

    fn redraw(&mut self) -> Result<()> {
        let changes = mem::take(self.scene_changes.lock().unwrap().deref_mut());

        let surface_size = self.renderer.surface_size();
        let view_projection_matrix = self.camera.view_projection_matrix(Z_RANGE, surface_size);

        {
            let mut font_system = self.font_system.lock().unwrap();
            self.renderer.apply_changes(&mut font_system, changes)?;
        }

        match self.renderer.render_and_present(&view_projection_matrix) {
            Ok(_) => {}
            // Reconfigure the surface if lost
            // TODO: shouldn't we redraw here? Also, I think the renderer can do this, too.
            Err(wgpu::SurfaceError::Lost) => {
                self.renderer.reconfigure_surface();
            }
            // The system is out of memory, we should probably quit
            Err(wgpu::SurfaceError::OutOfMemory) => bail!("Out of memory"),
            // All other errors (Outdated, Timeout) should be resolved by the next frame
            Err(e) => {
                error!("{e:?}");
            }
        }

        Ok(())
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
}
