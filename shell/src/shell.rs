use std::{
    future::Future,
    mem,
    ops::DerefMut,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use cosmic_text::FontSystem;
use log::{error, info};
use massive_scene::{Director, SceneChange};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use wgpu::{Instance, InstanceDescriptor, PresentMode, Surface, SurfaceTarget, TextureFormat};
use winit::{
    application::ApplicationHandler,
    dpi::{self, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

use massive_animation::{Interpolatable, Interpolation, Tickery, Timeline};
use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::Renderer;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub async fn run<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // Robustness: may use unbounded channels.
    let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    let tickery = Arc::new(Tickery::new(Instant::now()));

    let application_context = ApplicationContext {
        event_receiver,
        event_loop_proxy,
        tickery: tickery.clone(),
    };

    let (result_tx, mut result_rx) = oneshot::channel();
    let _application_task = tokio::spawn(async move {
        let r = application(application_context).await;
        // Found no way to retrieve the result via JoinHandle, so a return via a onceshot channel
        // must do.
        result_tx
            .send(Some(r))
            .expect("Internal Error: Failed to set the application result");
    });

    // Event loop

    {
        let mut winit_context = WinitApplicationHandler {
            event_sender,
            tickery,
        };

        info!("Entering event loop");
        event_loop.run_app(&mut winit_context)?;
        info!("Exiting event loop");
    };

    // Check application's result
    if let Ok(Some(r)) = result_rx.try_recv() {
        info!("Application ended with {r:?}");
        r?;
    } else {
        // TODO: This should probably be an error, we want to the application to have full
        // control over the lifetime of the winit event loop.
        info!("Application did not end");
    }

    Ok(())
}

// TODO: if window gets closed, remove it from active_windows in the Shell3.
pub struct ShellWindow {
    /// `Arc` because this is shared with the renderer because it needs to invoke request_redraw(), too.
    window: Arc<Window>,
    // For creating surfaces, we need to communicate with the Shell.
    event_loop_proxy: EventLoopProxy<ShellRequest>,
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
    ) -> Result<(WindowRenderer, Director)> {
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
                &self.window.window,
            ),
        }
        .await;
        let (instance, surface) = instance_and_surface?;

        // DI: If we can access the ShellWindow, we don't need a clone of font_system or
        // event_loop_proxy here.
        WindowRenderer::new(
            self.window.clone(),
            instance,
            surface,
            font_system,
            camera,
            initial_size,
        )
        .await
    }

    /// Helper to create instance and surce.
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

pub struct WindowRenderer {
    window: Arc<Window>,
    font_system: Arc<Mutex<FontSystem>>,
    camera: Camera,
    scene_changes: Arc<Mutex<Vec<SceneChange>>>,
    renderer: Renderer,
}

#[must_use]
#[derive(Debug, Copy, Clone)]
pub enum ControlFlow {
    Exit,
    Continue,
}

impl WindowRenderer {
    pub fn font_system(&self) -> &Arc<Mutex<FontSystem>> {
        &self.font_system
    }

    async fn new(
        window: Arc<Window>,
        instance: wgpu::Instance,
        surface: Surface<'static>,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // TODO: use a rect here to be able to position the renderer!
        initial_size: PhysicalSize<u32>,
    ) -> Result<(WindowRenderer, Director)> {
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
            // 20250721:
            // Since the time we are rendering asynchronously, not bound to the main thread,
            // VSync seems to be fast enough on MacOS and also fixes the "wobbly" resizing.
            present_mode: PresentMode::AutoVsync,
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

        let director = Director::new(move |changes| {
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
}

enum ShellRequest {
    CreateWindow {
        // Box because of large size.
        attributes: Box<WindowAttributes>,
        on_created: oneshot::Sender<Result<Window>>,
    },
    /// Surfaces need to be created on the main thread on macOS when a window handle is provided.
    CreateSurface {
        instance: Instance,
        target: SurfaceTarget<'static>,
        on_created: oneshot::Sender<Result<Surface<'static>>>,
    },
}

#[derive(Debug)]
pub enum ShellEvent {
    WindowEvent(WindowId, WindowEvent),
}

impl ShellEvent {
    #[must_use]
    pub fn window_event_for(&self, window: &ShellWindow) -> Option<&WindowEvent> {
        match self {
            ShellEvent::WindowEvent(id, window_event) if *id == window.id() => Some(window_event),
            _ => None,
        }
    }
}

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

impl WindowRenderer {
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
    fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }
}

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}

/// The [`ApplicationContext`] is the connection to the runtinme. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
pub struct ApplicationContext {
    event_receiver: UnboundedReceiver<ShellEvent>,
    event_loop_proxy: EventLoopProxy<ShellRequest>,
    tickery: Arc<Tickery>,
}

impl ApplicationContext {
    pub async fn new_window(
        &self,
        inner_size: impl Into<dpi::Size>,
        _canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        #[cfg(target_arch = "wasm32")]
        assert!(
            _canvas_id.is_none(),
            "Rendering to a canvas isn't support yet"
        );
        let (on_created, when_created) = oneshot::channel();
        let attributes = WindowAttributes::default().with_inner_size(inner_size);
        self.event_loop_proxy
            .send_event(ShellRequest::CreateWindow {
                attributes: attributes.into(),
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;

        let window = when_created.await??;
        Ok(ShellWindow {
            window: window.into(),
            event_loop_proxy: self.event_loop_proxy.clone(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(unused)]
    fn new_window_ev(
        &self,
        event_loop: &ActiveEventLoop,
        inner_size: impl Into<dpi::Size>,
        _canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        let window =
            event_loop.create_window(WindowAttributes::default().with_inner_size(inner_size))?;
        Ok(ShellWindow {
            window: Arc::new(window),
            event_loop_proxy: self.event_loop_proxy.clone(),
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn new_window_ev(
        &self,
        event_loop: &ActiveEventLoop,
        // We don't set inner size, the canvas defines how large we render.
        _inner_size: impl Into<dpi::Size>,
        canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;

        let canvas_id = canvas_id.expect("Canvas id is needed for wasm targets");

        let canvas = web_sys::window()
            .expect("No Window")
            .document()
            .expect("No document")
            .query_selector(&format!("#{canvas_id}"))
            // what a shit-show here, why is the error not compatible with anyhow.
            .map_err(|err| anyhow::anyhow!(err.as_string().unwrap()))?
            .expect("No Canvas with a matching id found");

        let canvas: web_sys::HtmlCanvasElement = canvas
            .dyn_into()
            .map_err(|_| anyhow::anyhow!("Failed to cast to HtmlCanvasElement"))?;

        let window =
            event_loop.create_window(WindowAttributes::default().with_canvas(Some(canvas)))?;

        Ok(ShellWindow {
            window: Rc::new(window),
        })
    }

    /// Create a timeline with a starting value.
    pub fn timeline<T: Interpolatable + Send>(&self, value: T) -> Timeline<T> {
        self.tickery.timeline(value)
    }

    /// Create a timeline that is animating from a starting value to a target value.
    pub fn animation<T: Interpolatable + 'static + Send>(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Timeline<T> {
        let mut timeline = self.tickery.timeline(value);
        timeline.animate_to(target_value, duration, interpolation);
        timeline
    }

    /// Waits for a shell event and updates all timelines if a [`ShellEvent::ApplyAnimations`] is
    /// received.
    pub async fn wait_for_event(&mut self) -> Result<ShellEvent> {
        let event = self.event_receiver.recv().await;
        let Some(event) = event else {
            // This means that the shell stopped before the application ended, this should not
            // happen in normal situations.
            bail!("Internal Error: Shell shut down, no more events")
        };

        // if let ShellEvent::ApplyAnimations(tick) = event {
        //     // Animations may have been removed in the meantime, so we check for wants_ticks()...
        //     self.tickery.tick(tick);
        //     // Even if nothing happened, the event _must_ be forwarded to the application, because
        //     // it may need to apply final values now.
        // }

        Ok(event)
    }

    /// Wait for the next event of a specific window and forward it to the renderer if needed.
    pub async fn wait_for_window_event(&mut self, window: &ShellWindow) -> Result<WindowEvent> {
        match self.wait_for_event().await? {
            ShellEvent::WindowEvent(window_id, window_event) if window_id == window.id() => {
                Ok(window_event)
            }
            _ => {
                // TODO: Support this somehow.
                panic!("Received event from another window")
            }
        }
    }
}

struct WinitApplicationHandler {
    event_sender: UnboundedSender<ShellEvent>,
    tickery: Arc<Tickery>,
}

const ANIMATION_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

impl ApplicationHandler<ShellRequest> for WinitApplicationHandler {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Robustness: As recommended, wait for the resumed event before creating any window.
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellRequest) {
        match event {
            ShellRequest::CreateWindow {
                attributes,
                on_created,
            } => {
                let r = event_loop.create_window(*attributes);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
            ShellRequest::CreateSurface {
                instance,
                target,
                on_created,
            } => {
                let r = instance.create_surface(target);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event != WindowEvent::RedrawRequested {
            info!("{event:?}");
        }

        self.send_event(event_loop, ShellEvent::WindowEvent(window_id, event))
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, shell_event: ShellEvent) {
        if let Err(_e) = self.event_sender.send(shell_event) {
            // Don't log when we are already exiting.
            if !event_loop.exiting() {
                info!("Receiver for events dropped, exiting event loop: {_e:?}");
                event_loop.exit();
            }
        }
    }
}
