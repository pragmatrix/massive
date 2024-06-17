use std::{
    cell::RefCell,
    future::Future,
    ptr,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{self, Waker},
};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use futures::{task::ArcWake, FutureExt};
use log::{error, info};
use massive_scene::{Director, SceneChange};
use tokio::{
    sync::{
        mpsc::{channel, error::TryRecvError, Receiver, Sender},
        oneshot,
    },
    task::LocalSet,
};
use wgpu::{Instance, InstanceDescriptor, PresentMode, Surface, SurfaceTarget, TextureFormat};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::Renderer;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub struct Shell3 {
    pub font_system: Arc<Mutex<text::FontSystem>>,
}

// TODO: if window gets closed, remove it from active_windows in the Shell3.
pub struct ShellWindow {
    window: Window,
}

impl ShellWindow {
    fn request_redraw(&self) {
        self.window.request_redraw()
    }

    fn inner_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }
}

pub struct WindowRenderer<'window> {
    font_system: Arc<Mutex<FontSystem>>,
    window: &'window ShellWindow,
    camera: Camera,
    scene_changes: Receiver<Vec<SceneChange>>,
    renderer: Renderer<'window>,
}

#[must_use]
#[derive(Debug, Copy, Clone)]
pub enum ControlFlow {
    Exit,
    Continue,
}

impl<'window> WindowRenderer<'window> {
    pub fn handle_event(&mut self, event: ShellEvent) -> ControlFlow {
        match event {
            ShellEvent::WindowEvent(window_id, window_event) => {
                if self.window.window.id() == window_id {
                    return self.handle_window_event(window_event);
                }
            }
            ShellEvent::RequestRedraw(_) => {}
        }
        ControlFlow::Continue
    }

    fn handle_window_event(&mut self, window_event: WindowEvent) -> ControlFlow {
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
                return self.redraw();
            }
            _ => {}
        }
        ControlFlow::Continue
    }

    pub fn redraw(&mut self) -> ControlFlow {
        let mut changes = Vec::new();

        // Pull all scene changes out of the channel.
        loop {
            match self.scene_changes.try_recv() {
                Ok(new_changes) => {
                    changes.extend(new_changes);
                    continue;
                }
                Err(TryRecvError::Empty) => {
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    info!("Scene Changes channel disconnected, recommending exit control flow to the caller");
                    return ControlFlow::Exit;
                }
            }
        }

        // let surface_matrix = self.renderer.surface_matrix();
        let surface_size = self.renderer.surface_size();
        let view_projection_matrix = self.camera.view_projection_matrix(Z_RANGE, surface_size);

        // let surface_view_matrix = surface_matrix * view_projection_matrix;

        {
            let mut font_system = self.font_system.lock().unwrap();
            self.renderer
                .apply_changes(&mut font_system, changes)
                .expect("Render preparations failed");
        }

        // TODO: pass primitives as value.
        match self.renderer.render_and_present(&view_projection_matrix) {
            Ok(_) => {}
            // Reconfigure the surface if lost
            // TODO: shouldn't we redraw here? Also, I think the renderer can do this, too.
            Err(wgpu::SurfaceError::Lost) => {
                self.renderer.reconfigure_surface();
            }
            // The system is out of memory, we should probably quit
            Err(wgpu::SurfaceError::OutOfMemory) => return ControlFlow::Exit,
            // All other errors (Outdated, Timeout) should be resolved by the next frame
            Err(e) => {
                error!("{:?}", e);
            }
        }

        ControlFlow::Continue
    }
}

enum ShellEvent {
    WindowEvent(WindowId, WindowEvent),
    RequestRedraw(WindowId),
}

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

impl Shell3 {
    pub fn event_loop() -> Result<EventLoop<ShellEvent3>> {
        Ok(EventLoop::with_user_event().build()?)
    }

    // Creating some of the wgpu types requires async code
    // TODO: We need the `FontSystem` only while rendering.
    pub async fn new(font_system: Arc<Mutex<FontSystem>>) -> Result<Shell3> {
        Ok(Self { font_system })
    }

    pub async fn run<R: Future<Output = Result<()>> + 'static>(
        &mut self,
        event_loop: EventLoop<ShellEvent3>,
        application: impl FnOnce(ApplicationContext3) -> R + 'static,
    ) -> Result<()> {
        // Spawn application.

        // TODO: may use unbounded channels.
        let (event_sender, event_receiver) = channel(256);

        let proxy = event_loop.create_proxy();

        let active_event_loop: Rc<RefCell<*const ActiveEventLoop>> =
            Rc::new(RefCell::new(ptr::null()));

        let application_context = ApplicationContext3 {
            font_system: self.font_system.clone(),
            event_receiver,
            active_event_loop: active_event_loop.clone(),
        };

        let local_set = LocalSet::new();
        let (result_tx, mut result_rx) = oneshot::channel();
        let _application_task = local_set.spawn_local(async move {
            let r = application(application_context).await;
            // Found no way to retrieve the result via JoinHandle, so this must do.
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
                shell: self,
                event_sender,
                active_event_loop,
                local_set,
                waker,
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
}

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

pub struct ApplicationContext3 {
    font_system: Arc<Mutex<FontSystem>>,
    event_receiver: Receiver<ShellEvent>,
    active_event_loop: Rc<RefCell<*const ActiveEventLoop>>,
}

impl ApplicationContext3 {
    pub fn create_window(&self, inner_size: PhysicalSize<u32>) -> Result<ShellWindow> {
        self.with_active_event_loop(|event_loop| {
            let window = event_loop
                .create_window(WindowAttributes::default().with_inner_size(inner_size))?;
            Ok(ShellWindow { window })
        })
    }

    fn with_active_event_loop<R>(
        &self,
        f: impl FnOnce(&ActiveEventLoop) -> Result<R>,
    ) -> Result<R> {
        let ptr = self.active_event_loop.borrow();
        let ptr = *ptr;
        if ptr.is_null() {
            panic!("Active event loop not set");
        }
        let active_event_loop = unsafe { &*ptr };
        f(active_event_loop)
    }

    pub async fn create_renderer<'window>(
        &self,
        window: &'window ShellWindow,
        // TODO: use a rect here to be able to position the renderer!
        initial_size: PhysicalSize<u32>,
        camera: Camera,
    ) -> Result<(WindowRenderer<'window>, Director)> {
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
                window,
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

        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|f| *f == PresentMode::Immediate)
            .unwrap_or(surface_caps.present_modes[0]);

        let alpha_mode = surface_caps.alpha_modes[0];

        info!(
            "Selecting present mode {:?}, alpha mode: {:?}, initial size: {:?}",
            present_mode, alpha_mode, initial_size,
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_size.width,
            height: initial_size.height,
            present_mode,
            // TODO: Select this explicitly
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);
        let renderer = Renderer::new(device, queue, surface, surface_config);

        // TODO: may use unbounded channels.
        let (scene_sender, scene_receiver) = channel::<Vec<SceneChange>>(256);

        let window_renderer = WindowRenderer {
            font_system: self.font_system.clone(),
            window,
            camera,
            scene_changes: scene_receiver,
            renderer,
        };

        // TODO: cause a redraw on the window.
        let director = Director::new(move |changes| Ok(scene_sender.try_send(changes)?));

        Ok((window_renderer, director))
    }
}

struct WinitApplicationHandler<'shell> {
    shell: &'shell mut Shell3,
    event_sender: Sender<ShellEvent>,
    active_event_loop: Rc<RefCell<*const ActiveEventLoop>>,
    local_set: LocalSet,
    waker: Arc<EventLoopWaker>,
}

impl ApplicationHandler<ShellEvent3> for WinitApplicationHandler<'_> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: ShellEvent3) {
        match event {
            ShellEvent3::RequestRedraw(window_id) => {
                if self
                    .event_sender
                    .try_send(ShellEvent::RequestRedraw(window_id))
                    .is_err()
                {
                    info!("Receiver for events dropped, exiting event loop");
                    event_loop.exit();
                    return;
                };
            }
            ShellEvent3::WakeUpApplication => {}
        }
        self.drive_application(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        info!("{:?}", event);

        match self
            .event_sender
            .try_send(ShellEvent::WindowEvent(window_id, event))
        {
            Err(_e) => {
                info!("Receiver for events dropped, exiting event loop");
                event_loop.exit();
            }
            Ok(()) => self.drive_application(event_loop),
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.drive_application(event_loop);
    }
}

impl WinitApplicationHandler<'_> {
    fn drive_application(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let waker_ref = futures::task::waker_ref(&self.waker);
        let mut context = task::Context::from_waker(&waker_ref);

        *self.active_event_loop.borrow_mut() = event_loop;

        if self.local_set.poll_unpin(&mut context).is_ready() {
            event_loop.exit();
        }

        *self.active_event_loop.borrow_mut() = ptr::null();
    }
}

#[derive(Debug)]
pub enum ShellEvent3 {
    WakeUpApplication,
    RequestRedraw(WindowId),
}

struct EventLoopWaker {
    proxy: EventLoopProxy<ShellEvent3>,
    waker: Mutex<Option<Waker>>,
}

impl ArcWake for EventLoopWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        if let Ok(mut waker_guard) = arc_self.waker.lock() {
            if let Some(waker) = waker_guard.take() {
                waker.wake();
            }
        }
        let _ = arc_self.proxy.send_event(ShellEvent3::WakeUpApplication);
    }
}
