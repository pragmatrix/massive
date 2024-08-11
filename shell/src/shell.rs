use std::{
    cell::RefCell,
    future::Future,
    ptr,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use cosmic_text::FontSystem;
use futures::{task::ArcWake, FutureExt};
use log::{debug, error, info};
use massive_scene::{Director, SceneChange};
use tokio::{
    sync::{
        mpsc::{channel, Receiver, Sender},
        oneshot,
    },
    task::LocalSet,
};
use wgpu::{Instance, InstanceDescriptor, PresentMode, Surface, SurfaceTarget, TextureFormat};
use winit::{
    application::ApplicationHandler,
    dpi::{self, PhysicalSize},
    event::{StartCause, WindowEvent},
    event_loop::{self, ActiveEventLoop, EventLoop, EventLoopProxy},
    monitor::MonitorHandle,
    window::{Window, WindowAttributes, WindowId},
};

use massive_animation::{Interpolatable, Tickery, Timeline};
use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::Renderer;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub async fn run<R: Future<Output = Result<()>> + 'static>(
    application: impl FnOnce(ApplicationContext) -> R + 'static,
) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // TODO: may use unbounded channels.
    let (event_sender, event_receiver) = channel(256);

    let proxy = event_loop.create_proxy();

    let active_event_loop: Rc<RefCell<*const ActiveEventLoop>> = Rc::new(RefCell::new(ptr::null()));

    let tickery = Rc::new(Tickery::default());

    let application_context = ApplicationContext {
        event_receiver,
        active_event_loop: active_event_loop.clone(),
        tickery: tickery.clone(),
    };

    let local_set = LocalSet::new();
    let (result_tx, mut result_rx) = oneshot::channel();
    let _application_task = local_set.spawn_local(async move {
        let r = application(application_context).await;
        // Found no way to retrieve the result via JoinHandle, so a return via a onceshot channel
        // must do.
        result_tx
            .send(Some(r))
            .expect("Internal Error: Failed to set the application result");
    });

    // Event loop

    {
        // Shared state to wake the event loop
        let waker = Arc::new(EventLoopWaker {
            proxy: proxy.clone(),
            waker: Mutex::new(None),
        });

        let mut winit_context = WinitApplicationHandler {
            event_sender,
            active_event_loop,
            local_set,
            waker,

            tickery,
        };

        info!("Entering event loop");
        event_loop.run_app(&mut winit_context)?;
        info!("Exiting event loop");
    }

    // Check application's result
    if let Ok(Some(r)) = result_rx.try_recv() {
        info!("Application ended with {:?}", r);
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
    // Rc because the renderer needs to invoke request_redraw()
    window: Rc<Window>,
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
        // DI: If we can access the ShellWindow, we don't need a clone of font_system or
        // event_loop_proxy here.
        WindowRenderer::new(self, font_system, camera, initial_size).await
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

pub struct WindowRenderer<'window> {
    window: &'window ShellWindow,
    font_system: Arc<Mutex<FontSystem>>,
    camera: Camera,
    scene_changes: Rc<RefCell<Vec<SceneChange>>>,
    renderer: Renderer<'window>,
}

#[must_use]
#[derive(Debug, Copy, Clone)]
pub enum ControlFlow {
    Exit,
    Continue,
}

impl<'window> WindowRenderer<'window> {
    pub fn font_system(&self) -> &Arc<Mutex<FontSystem>> {
        &self.font_system
    }

    async fn new(
        window: &ShellWindow,
        font_system: Arc<Mutex<FontSystem>>,
        camera: Camera,
        // TODO: use a rect here to be able to position the renderer!
        initial_size: PhysicalSize<u32>,
    ) -> Result<(WindowRenderer, Director)> {
        let instance_and_surface = WindowRenderer::create_instance_and_surface(
            InstanceDescriptor::default(),
            // Use this for testing webgl:
            // InstanceDescriptor {
            //     backends: wgpu::Backends::GL,
            //     ..InstanceDescriptor::default()
            // },
            &window.window,
        );
        // On wasm, attempt to fall back to webgl
        #[cfg(target_arch = "wasm32")]
        let instance_and_surface = match instance_and_surface {
            Ok(_) => instance_and_surface,
            Err(_) => Self::create_instance_and_surface(
                InstanceDescriptor {
                    backends: wgpu::Backends::GL,
                    ..InstanceDescriptor::default()
                },
                &window.window,
            ),
        };
        let (instance, surface) = instance_and_surface?;

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
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    // May be wrong, see: <https://github.com/gfx-rs/wgpu/blob/1144b065c4784d769d59da2f58f5aa13212627b0/examples/src/hello_triangle/mod.rs#L33-L34>
                    required_limits: adapter.limits(),
                    label: None,
                },
                None, // Trace path
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        // Don't use srgb now, colors are specified in linear rgb space.
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);

        info!("Surface format: {:?}", surface_format);

        info!("Available present modes: {:?}", surface_caps.present_modes);

        let alpha_mode = surface_caps.alpha_modes[0];

        info!(
            "Selecting alpha mode: {:?}, initial size: {:?}",
            alpha_mode, initial_size,
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_size.width,
            height: initial_size.height,
            present_mode: PresentMode::AutoNoVsync,
            // TODO: Select explicitly
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);
        let renderer = Renderer::new(device, queue, surface, surface_config);

        let scene_changes = Rc::new(RefCell::new(Vec::new()));

        let window_renderer = WindowRenderer {
            window,
            font_system,
            camera,
            scene_changes: scene_changes.clone(),
            renderer,
        };

        let window = window.window.clone();

        let director = Director::new(move |changes| {
            // Since we are the only one pushing to the renderer, we can invoke a request redraw
            // only once as soon `scene_changes` switch from empty to non-empty.
            let request_redraw = {
                let mut scene_changes = scene_changes.borrow_mut();
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
                info!("{:?}", window_event);
                self.renderer
                    .resize_surface((physical_size.width, physical_size.height));
                self.window.request_redraw()
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let new_inner_size = self.window.inner_size();
                self.renderer
                    .resize_surface((new_inner_size.width, new_inner_size.height));
                self.window.request_redraw()
            }
            WindowEvent::RedrawRequested => {
                self.redraw()?;
            }
            _ => {}
        }

        Ok(())
    }

    fn redraw(&mut self) -> Result<()> {
        let changes = self.scene_changes.take();

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
                error!("{:?}", e);
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ShellEvent {
    WindowEvent(WindowId, WindowEvent),
    ApplyAnimations,
}

impl ShellEvent {
    #[must_use]
    pub fn window_event_for(&self, window: &ShellWindow) -> Option<&WindowEvent> {
        match self {
            ShellEvent::WindowEvent(id, window_event) if *id == window.id() => Some(window_event),
            _ => None,
        }
    }

    #[must_use]
    pub fn apply_animations(&self) -> bool {
        matches!(self, ShellEvent::ApplyAnimations)
    }
}

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

impl<'window> WindowRenderer<'window> {
    fn create_instance_and_surface(
        instance_descriptor: InstanceDescriptor,
        surface_target: &Window,
    ) -> Result<(Instance, Surface<'_>)> {
        let instance = wgpu::Instance::new(instance_descriptor);

        let surface_target: SurfaceTarget = surface_target.into();
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

        let surface = instance.create_surface(surface_target)?;
        Ok((instance, surface))
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
    fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }
}

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}

/// The [`ApplicationContext`] is the connection to the window. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.

pub struct ApplicationContext {
    event_receiver: Receiver<ShellEvent>,
    active_event_loop: Rc<RefCell<*const ActiveEventLoop>>,
    tickery: Rc<Tickery>,
}

impl ApplicationContext {
    pub fn new_window(
        &self,
        inner_size: impl Into<dpi::Size>,
        canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        self.with_active_event_loop(|event_loop| {
            self.new_window_ev(event_loop, inner_size, canvas_id)
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn new_window_ev(
        &self,
        event_loop: &ActiveEventLoop,
        inner_size: impl Into<dpi::Size>,
        _canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        let window =
            event_loop.create_window(WindowAttributes::default().with_inner_size(inner_size))?;
        Ok(ShellWindow {
            window: Rc::new(window),
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

    pub fn primary_monitor(&self) -> Option<MonitorHandle> {
        self.with_active_event_loop(|event_loop| event_loop.primary_monitor())
    }

    pub fn timeline<T: Interpolatable>(&self, value: T) -> Timeline<T> {
        self.tickery.timeline(value)
    }

    /// Executes a lambda when a [`ActiveEventLoop`] reference is available. I.e. the code currently
    /// running is run inside the winit event loop.
    ///
    /// Panics if the current code does not execute inside the winit event loop.
    fn with_active_event_loop<R>(&self, f: impl FnOnce(&ActiveEventLoop) -> R) -> R {
        let ptr = self.active_event_loop.borrow();
        let ptr = *ptr;
        if ptr.is_null() {
            panic!("Active event loop not set");
        }
        let active_event_loop = unsafe { &*ptr };
        f(active_event_loop)
    }

    pub async fn wait_for_event(&mut self) -> Result<ShellEvent> {
        let event = self.event_receiver.recv().await;
        let Some(event) = event else {
            // This means that the shell stopped before the application ended, this should not
            // happen in normal situations.
            bail!("Internal Error: Shell shut down, no more events")
        };

        Ok(event)
    }

    /// Retrieve the next window event and forward it to the renderer if needed.
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
    event_sender: Sender<ShellEvent>,
    active_event_loop: Rc<RefCell<*const ActiveEventLoop>>,
    local_set: LocalSet,
    waker: Arc<EventLoopWaker>,

    tickery: Rc<Tickery>,
}

const ANIMATION_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

impl ApplicationHandler<Event> for WinitApplicationHandler {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::ResumeTimeReached {
                requested_resume, ..
            } => {
                self.tickery.tick(requested_resume);
                self.send_event(event_loop, ShellEvent::ApplyAnimations);
                if self.tickery.wants_ticks() {
                    event_loop.set_control_flow(event_loop::ControlFlow::WaitUntil(
                        requested_resume + ANIMATION_FRAME_DURATION,
                    ));
                } else {
                    debug!("Animation stopped");
                    // Winit does not stop sending ResumeTimeReached until the control flow gets
                    // changed.
                    event_loop.set_control_flow(event_loop::ControlFlow::Wait);
                }
            }
            StartCause::WaitCancelled {
                requested_resume: Some(requested_resume),
                ..
            } => {
                // Re-set a new wakeup time when the tickery has registrations.
                if self.tickery.wants_ticks() {
                    event_loop
                        .set_control_flow(event_loop::ControlFlow::WaitUntil(requested_resume));
                } else {
                    debug!("Animation stopped");
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::WakeUpApplication => {
                self.drive_application(event_loop);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // This was added for the logs example.
        if event != WindowEvent::RedrawRequested {
            info!("{:?}", event);
        }

        self.send_event(event_loop, ShellEvent::WindowEvent(window_id, event))
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drive_application(event_loop);
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, shell_event: ShellEvent) {
        match self.event_sender.try_send(shell_event) {
            Ok(()) => {
                // OO: Can't we wait until the event loop is about to wait and then drive the
                // application which pulls all new events? Effectively collecting all the events?
                self.drive_application(event_loop)
            }
            Err(_e) => {
                // Don't log when we are already exiting.
                if !event_loop.exiting() {
                    info!("Receiver for events dropped, exiting event loop");
                    event_loop.exit();
                }
            }
        }
    }

    fn drive_application(&mut self, event_loop: &ActiveEventLoop) {
        let wanted_ticks = self.tickery.wants_ticks();

        {
            let waker_ref = futures::task::waker_ref(&self.waker);
            let mut context = std::task::Context::from_waker(&waker_ref);

            *self.active_event_loop.borrow_mut() = event_loop;

            if self.local_set.poll_unpin(&mut context).is_ready() {
                event_loop.exit();
            }

            *self.active_event_loop.borrow_mut() = ptr::null();
        }

        // Transitioning into an animation state?
        if !wanted_ticks && self.tickery.wants_ticks() {
            let now = Instant::now();
            event_loop.set_control_flow(event_loop::ControlFlow::WaitUntil(
                now + ANIMATION_FRAME_DURATION,
            ));
            debug!("Animation started");
        }
    }
}

#[derive(Debug)]
enum Event {
    WakeUpApplication,
}

struct EventLoopWaker {
    proxy: EventLoopProxy<Event>,
    waker: Mutex<Option<std::task::Waker>>,
}

impl ArcWake for EventLoopWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        if let Ok(mut waker_guard) = arc_self.waker.lock() {
            if let Some(waker) = waker_guard.take() {
                waker.wake();
            }
        }
        let _ = arc_self.proxy.send_event(Event::WakeUpApplication);
    }
}
